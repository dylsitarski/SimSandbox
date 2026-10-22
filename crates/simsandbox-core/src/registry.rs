//! The block-type registry: builds Blocks from a type name and method
//! settings, so models can be loaded from files (ADR 0007).
//!
//! Domain crates register their types; the kernel only stores factories and
//! knows nothing about file formats.

use crate::block::{Block, Domain};
use crate::fusion::GroupSolver;
use crate::{Result, SimError};
use std::collections::BTreeMap;

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
