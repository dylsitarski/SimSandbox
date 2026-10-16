//! The block-type registry: builds Blocks from a type name and method
//! settings, so models can be loaded from files (ADR 0007).
//!
//! Domain crates register their types; the kernel only stores factories and
//! knows nothing about file formats.

use crate::block::Block;
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

/// Builds a Block named `name` running the given method.
pub type BlockFactory = fn(name: &str, method: &MethodSettings) -> Result<Box<dyn Block>>;

/// Maps type names (namespaced by domain, e.g. `eq.lorenz`) to factories.
#[derive(Default)]
pub struct Registry {
    factories: BTreeMap<String, BlockFactory>,
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
        method: &MethodSettings,
    ) -> Result<Box<dyn Block>> {
        let factory = self
            .factories
            .get(type_name)
            .ok_or_else(|| SimError::UnknownBlockType(type_name.into()))?;
        factory(name, method)
    }

    pub fn contains(&self, type_name: &str) -> bool {
        self.factories.contains_key(type_name)
    }

    /// Registered type names, sorted.
    pub fn type_names(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }
}
