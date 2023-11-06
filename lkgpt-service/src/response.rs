use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerMsg<T> {
    data: Option<T>,
    error: Option<T>,
}

impl<T> ServerMsg<T> {
    pub fn data(data: T) -> Self {
        Self {
            data: Some(data),
            error: None,
        }
    }

    pub fn error(error: T) -> Self {
        Self {
            data: None,
            error: Some(error),
        }
    }
}

pub enum CommonResponses {
    MethodNotAllowed,
}

impl CommonResponses {
    pub fn json(&self) -> ServerMsg<String> {
        match self {
            CommonResponses::MethodNotAllowed => ServerMsg::error("Method not allowed".to_string()),
        }
    }
}
