use thiserror::Error as ThisError;

#[derive(ThisError, Debug, PartialEq)]
pub enum Error {
    #[error("Id not found")]
    IdNotFound,

    #[error("Timeout")]
    Timeout,
}
