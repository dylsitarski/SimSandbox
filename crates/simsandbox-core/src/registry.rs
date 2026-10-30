//! The block-type registry: builds Blocks from a type name and method
//! settings, so models can be loaded from files (ADR 0007).
//!
//! Domain crates register their types; the kernel only stores factories and
//! knows nothing about file formats.

use crate::block::{Block, Domain};
use crate::fusion::GroupSolver;
use crate::{Result, SimError};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

thread_local! {
    static FOLDER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Run `f` with `folder` as where blocks on this thread read the files
/// their settings name when their configuration gives no folder (ADR
/// 0063): headless tools wrap a command on a model file in the file's
/// folder, so every function that loads the model's text finds its data.
pub fn in_folder<T>(folder: Option<&Path>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            FOLDER.with(|c| *c.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(set_folder(folder.map(Into::into)));
    f()
}

/// Set the folder of [`in_folder`] on this thread until set again, for a
/// caller that cannot wrap its work in a closure (Python); returns the
/// folder it replaces.
pub fn set_folder(folder: Option<PathBuf>) -> Option<PathBuf> {
    FOLDER.with(|c| c.replace(folder))
}

/// The folder set by [`in_folder`] on this thread, if any.
pub fn current_folder() -> Option<PathBuf> {
    FOLDER.with(|c| c.borrow().clone())
}

/// The method a Block should run, as written in a model file: a method name
/// plus numeric options, e.g. `rk4` with `substeps = 4`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MethodSettings {
    pub name: String,
    pub options: BTreeMap<String, f64>,
}

impl MethodSettings {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            options: BTreeMap::new(),
        }
    }

    pub fn with(mut self, option: &str, value: f64) -> Self {
        self.options.insert(option.into(), value);
        self
    }
}

/// What a model file says about one Block besides its name and type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockConfig {
    pub method: MethodSettings,
    /// Domain-specific lines that define the Block, e.g. the equations of an
    /// `eq.equations` block (ADR 0017). Empty for fixed types such as
    /// `eq.lorenz`.
    pub definition: Vec<String>,
    /// The order agents act in within a step (ADR 0034), for agent types:
    /// e.g. `random_order`. `None` takes the type's default.
    pub update: Option<String>,
    /// Named text settings, e.g. a unit conversion's `from` and `to`
    /// (ADR 0045). A type rejects settings it does not have.
    pub settings: BTreeMap<String, String>,
    /// The folder file names in settings are read from: the model file's
    /// (ADR 0063). `None` for a model not saved in a folder; a type then
    /// accepts only absolute file names.
    pub folder: Option<PathBuf>,
}

impl BlockConfig {
    /// Where the file named in a setting is: as written if absolute, else
    /// in [`BlockConfig::folder`] or, without one, the folder set by
    /// [`in_folder`].
    pub fn file(&self, setting: &str) -> Option<Result<PathBuf>> {
        let name = self.settings.get(setting)?;
        let path = PathBuf::from(name);
        Some(if path.is_absolute() {
            Ok(path)
        } else if let Some(folder) = self.folder.clone().or_else(current_folder) {
            Ok(folder.join(path))
        } else {
            Err(SimError::InvalidSetting {
                setting: format!("settings.{setting}"),
                reason: format!(
                    "'{name}' is read from the model's folder: save the model first, or give a full path"
                ),
            })
        })
    }
}

/// What a block type is and how it is configured, for discovery by people
/// and AI agents (ADR 0039): shown by `simsandbox describe`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypeInfo {
    /// One or two sentences on what the type models.
    pub summary: String,
    /// The methods it accepts, the default first, each with the names of
    /// its numeric options.
    pub methods: Vec<(String, Vec<String>)>,
    /// For types defined by lines (typed equations), an example definition;
    /// `None` for fixed types.
    pub definition: Option<Vec<String>>,
    /// Named settings and their choices, e.g. `update`.
    pub settings: Vec<(String, Vec<String>)>,
    /// Settings that name files (ADR 0063): read from the model's folder,
    /// and part of a run's model hash by their contents.
    pub files: Vec<String>,
}

impl TypeInfo {
    pub fn new(summary: &str) -> Self {
        Self {
            summary: summary.into(),
            ..Self::default()
        }
    }

    pub fn method(mut self, name: &str, options: &[&str]) -> Self {
        self.methods
            .push((name.into(), options.iter().map(|o| o.to_string()).collect()));
        self
    }

    pub fn definition(mut self, lines: &[&str]) -> Self {
        self.definition = Some(lines.iter().map(|l| l.to_string()).collect());
        self
    }

    pub fn setting(mut self, name: &str, choices: &[&str]) -> Self {
        self.settings
            .push((name.into(), choices.iter().map(|c| c.to_string()).collect()));
        self
    }

    /// A setting that names a file.
    pub fn file(mut self, name: &str) -> Self {
        self.files.push(name.into());
        self
    }
}

/// Builds a Block named `name` from its configuration.
pub type BlockFactory = fn(name: &str, config: &BlockConfig) -> Result<Box<dyn Block>>;

/// Builds the method that advances a fused group (ADR 0023).
pub type GroupSolverFactory = fn(method: &MethodSettings) -> Result<Box<dyn GroupSolver>>;

/// Maps type names (namespaced by domain, e.g. `eq.lorenz`) to factories,
/// and domains to the methods that solve their fused groups.
#[derive(Default)]
pub struct Registry {
    factories: BTreeMap<String, BlockFactory>,
    info: BTreeMap<String, TypeInfo>,
    group_solvers: Vec<(Domain, GroupSolverFactory)>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a type. Registering the same name twice is an error.
    pub fn register(&mut self, type_name: &str, factory: BlockFactory) -> Result<()> {
        if self.factories.contains_key(type_name) {
            return Err(SimError::DuplicateBlockType(type_name.into()));
        }
        self.factories.insert(type_name.into(), factory);
        Ok(())
    }

    /// Describe a registered type (ADR 0039).
    pub fn describe(&mut self, type_name: &str, info: TypeInfo) -> Result<()> {
        if !self.factories.contains_key(type_name) {
            return Err(SimError::UnknownBlockType(type_name.into()));
        }
        self.info.insert(type_name.into(), info);
        Ok(())
    }

    /// What a type is, if described.
    pub fn info(&self, type_name: &str) -> Option<&TypeInfo> {
        self.info.get(type_name)
    }

    /// A Block of a described type built with its default method and
    /// example definition, to read its ports, parameters and labels.
    pub fn example(&self, type_name: &str) -> Result<Box<dyn Block>> {
        let info = self
            .info(type_name)
            .ok_or_else(|| SimError::UnknownBlockType(type_name.into()))?;
        let method = info
            .methods
            .first()
            .map_or_else(MethodSettings::default, |(m, _)| MethodSettings::new(m));
        let config = BlockConfig {
            method,
            definition: info.definition.clone().unwrap_or_default(),
            update: None,
            ..Default::default()
        };
        self.build(type_name, "example", &config)
    }

    /// Build a Block of a registered type.
    pub fn build(
        &self,
        type_name: &str,
        name: &str,
        config: &BlockConfig,
    ) -> Result<Box<dyn Block>> {
        let factory = self
            .factories
            .get(type_name)
            .ok_or_else(|| SimError::UnknownBlockType(type_name.into()))?;
        // No method given: the type's default, its first (ADR 0045). Types
        // defined by lines choose from what the lines describe.
        if config.method.name.is_empty() {
            let fixed = self.info(type_name).filter(|i| i.definition.is_none());
            if let Some((m, _)) = fixed.and_then(|i| i.methods.first()) {
                let config = BlockConfig {
                    method: MethodSettings::new(m),
                    ..config.clone()
                };
                return factory(name, &config);
            }
        }
        factory(name, config)
    }

    /// Register how a domain solves fused groups. Once per domain.
    pub fn register_group_solver(
        &mut self,
        domain: Domain,
        factory: GroupSolverFactory,
    ) -> Result<()> {
        if self.group_solvers.iter().any(|(d, _)| *d == domain) {
            return Err(SimError::DuplicateBlockType(format!(
                "group solver for {domain:?}"
            )));
        }
        self.group_solvers.push((domain, factory));
        Ok(())
    }

    /// The method for a fused group of this domain, or `None` if the
    /// domain cannot fuse.
    pub fn group_solver(
        &self,
        domain: Domain,
        method: &MethodSettings,
    ) -> Option<Result<Box<dyn GroupSolver>>> {
        self.group_solvers
            .iter()
            .find(|(d, _)| *d == domain)
            .map(|(_, f)| f(method))
    }

    pub fn contains(&self, type_name: &str) -> bool {
        self.factories.contains_key(type_name)
    }

    /// Registered type names, sorted.
    pub fn type_names(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }
}
