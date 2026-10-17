use std::fmt;

/// Errors raised by the kernel and by Blocks.
#[derive(Debug, Clone, PartialEq)]
pub enum SimError {
    /// A unit string could not be parsed.
    UnitParse(String),
    /// Two ports were connected whose units have different dimensions.
    UnitMismatch { from: String, to: String },
    /// Two ports were connected whose shapes differ.
    ShapeMismatch { from: String, to: String },
    /// A connection referenced a Block or port that does not exist,
    /// or went the wrong way (input to output).
    BadConnection(String),
    /// An input port was left unconnected.
    UnconnectedInput(String),
    /// A Block failed while initialising or stepping.
    Block(String),
    /// A Block has no parameter with this name.
    UnknownParam { block: String, name: String },
    /// A parameter value is not finite (NaN or infinite).
    InvalidParam { name: String, value: f64 },
    /// No block type is registered under this name.
    UnknownBlockType(String),
    /// A block type was registered twice.
    DuplicateBlockType(String),
    /// Method settings name an unknown method or carry invalid options.
    InvalidMethod(String),
    /// A Block's definition (e.g. its equations) is invalid.
    InvalidDefinition(String),
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimError::UnitParse(s) => write!(f, "cannot parse unit '{s}'"),
            SimError::UnitMismatch { from, to } => {
                write!(f, "unit mismatch: cannot connect [{from}] to [{to}]")
            }
            SimError::ShapeMismatch { from, to } => {
                write!(f, "shape mismatch: cannot connect {from} to {to}")
            }
            SimError::BadConnection(s) => write!(f, "bad connection: {s}"),
            SimError::UnconnectedInput(s) => write!(f, "input '{s}' is not connected"),
            SimError::Block(s) => write!(f, "block error: {s}"),
            SimError::UnknownParam { block, name } => {
                write!(f, "'{block}' has no parameter '{name}'")
            }
            SimError::InvalidParam { name, value } => {
                write!(f, "parameter '{name}' must be finite, got {value}")
            }
            SimError::UnknownBlockType(s) => write!(f, "unknown block type '{s}'"),
            SimError::DuplicateBlockType(s) => write!(f, "block type '{s}' is already registered"),
            SimError::InvalidMethod(s) => write!(f, "invalid method: {s}"),
            SimError::InvalidDefinition(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for SimError {}

pub type Result<T> = std::result::Result<T, SimError>;
