use ts_rs::TS;
use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! string_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[serde(transparent)]
        #[ts(type = "string")]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

string_newtype!(ApiName);
string_newtype!(ModelId);
string_newtype!(ModelName);
string_newtype!(ProviderName);
string_newtype!(SessionId);
string_newtype!(ToolCallId);
string_newtype!(ToolName);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct JsonSchema(pub Value);

impl JsonSchema {
    pub fn new(value: Value) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ToolArguments(pub Value);

impl ToolArguments {
    pub fn new(value: Value) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ToolDetails(pub Value);

impl ToolDetails {
    pub fn new(value: Value) -> Self {
        Self(value)
    }
}
