use sp_runtime::{transaction_validity::TransactionValidityError, DispatchError};

use crate::metadata::{Metadata, MetadataError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Initial websocket handshake error.
    #[error("Rpc error: {0}")]
    WsHandshake(#[from] jsonrpsee::transport::ws::WsNewDnsError),
    // Rpc error.
    #[error("rpc error: {0}")]
    Rpc(#[from] jsonrpsee::client::RequestError),
    /// Json serialization/deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Scale codec error.
    #[error("scale codec error: {0}")]
    Codec(#[from] codec::Error),
    /// Metadata error.
    #[error("Metadata error: {0}")]
    Metadata(#[from] MetadataError),
    /// Type size unavailable while decoding event.
    #[error("Type size unavailable while decoding event: {0:?}")]
    TypeSizeUnavailable(String),
    /// Transaction validity error.
    #[error("Transaction validity error: {0:?}")]
    TxValidity(TransactionValidityError),
    /// Runtime error.
    #[error("Runtime error: {0}")]
    Runtime(#[from] RuntimeError),
    /// Other error.
    #[error("Other error: {0}")]
    Other(String),
}

impl From<TransactionValidityError> for Error {
    fn from(error: TransactionValidityError) -> Self {
        Error::TxValidity(error)
    }
}

impl From<String> for Error {
    fn from(err: String) -> Self {
        Error::Other(err)
    }
}

impl From<&str> for Error {
    fn from(err: &str) -> Self {
        Error::Other(err.to_string())
    }
}

/// Runtime error.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RuntimeError {
    /// Module error.
    #[error("Runtime module error: {0}")]
    Module(ModuleError),
    /// Bad origin.
    #[error("Bad origin: throw by ensure_signed, ensure_root or ensure_none.")]
    BadOrigin,
    /// Cannot lookup.
    #[error("Cannot lookup some information required to validate the transaction.")]
    CannotLookup,
    /// Other error.
    #[error("Other error: {0}")]
    Other(String),
}

impl RuntimeError {
    /// Converts a `DispatchError` into a subxt error.
    pub fn from_dispatch(metadata: &Metadata, error: DispatchError) -> Result<Self, Error> {
        match error {
            DispatchError::Module {
                index,
                error,
                message: _,
            } => {
                let module = metadata.module_with_errors(index)?;
                let error = module.error(error)?;
                Ok(Self::Module(ModuleError {
                    module: module.name().to_string(),
                    error: error.to_string(),
                }))
            }
            DispatchError::BadOrigin => Ok(Self::BadOrigin),
            DispatchError::CannotLookup => Ok(Self::CannotLookup),
            DispatchError::Other(msg) => Ok(Self::Other(msg.into())),
        }
    }
}

/// Module error.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{error} from {module}")]
pub struct ModuleError {
    pub module: String,
    pub error: String,
}
