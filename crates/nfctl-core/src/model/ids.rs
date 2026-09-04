use std::fmt;

use serde::{Deserialize, Serialize};

/// Why a string is not a valid Kubernetes name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidName {
    #[error("name is empty")]
    Empty,
    #[error("name is longer than {max} characters")]
    TooLong { max: usize },
    #[error("name must be lowercase alphanumerics or '-', starting and ending alphanumeric")]
    BadChars,
}

/// RFC 1123 label: `[a-z0-9]([-a-z0-9]*[a-z0-9])?`, at most 63 chars.
fn check_dns_label(s: &str, max: usize) -> Result<(), InvalidName> {
    if s.is_empty() {
        return Err(InvalidName::Empty);
    }
    if s.len() > max {
        return Err(InvalidName::TooLong { max });
    }
    let ok_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    let first_last_ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !s.chars().all(ok_char) {
        return Err(InvalidName::BadChars);
    }
    let (Some(first), Some(last)) = (s.chars().next(), s.chars().last()) else {
        return Err(InvalidName::Empty);
    };
    if !first_last_ok(first) || !first_last_ok(last) {
        return Err(InvalidName::BadChars);
    }
    Ok(())
}

macro_rules! name_type {
    ($(#[$doc:meta])* $name:ident, max = $max:expr) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap.
            pub fn new(s: impl Into<String>) -> Result<Self, InvalidName> {
                let s = s.into();
                check_dns_label(&s, $max)?;
                Ok(Self(s))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidName;
            fn try_from(s: String) -> Result<Self, InvalidName> {
                Self::new(s)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = InvalidName;
            fn try_from(s: &str) -> Result<Self, InvalidName> {
                Self::new(s)
            }
        }

        impl From<$name> for String {
            fn from(n: $name) -> String {
                n.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.0)
            }
        }
    };
}

name_type!(
    /// A Kubernetes namespace.
    Namespace, max = 63
);
name_type!(
    /// A Numaflow `Pipeline` name.
    PipelineName, max = 63
);
name_type!(
    /// A vertex name within a pipeline.
    VertexName, max = 63
);
name_type!(
    /// An `InterStepBufferService` name.
    IsbName, max = 63
);
name_type!(
    /// A pod name.
    PodName, max = 253
);
name_type!(
    /// A container name within a pod.
    ContainerName, max = 63
);
name_type!(
    /// An inter-step buffer name as reported by the daemon.
    BufferName, max = 253
);

impl Namespace {
    /// The Kubernetes default namespace.
    #[must_use]
    pub fn default_ns() -> Self {
        Self("default".to_owned())
    }
}

/// Namespace + name: the identity of a pipeline in a cluster.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipelineKey {
    pub namespace: Namespace,
    pub name: PipelineName,
}

impl PipelineKey {
    #[must_use]
    pub fn new(namespace: Namespace, name: PipelineName) -> Self {
        Self { namespace, name }
    }
}

impl fmt::Display for PipelineKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_labels() {
        assert!(PipelineName::new("simple-pipeline").is_ok());
        assert!(PipelineName::new("a").is_ok());
        assert!(PipelineName::new("p1-2-3").is_ok());
    }

    #[test]
    fn rejects_invalid_labels() {
        assert_eq!(PipelineName::new(""), Err(InvalidName::Empty));
        assert_eq!(PipelineName::new("-lead"), Err(InvalidName::BadChars));
        assert_eq!(PipelineName::new("trail-"), Err(InvalidName::BadChars));
        assert_eq!(PipelineName::new("Upper"), Err(InvalidName::BadChars));
        assert_eq!(PipelineName::new("dot.name"), Err(InvalidName::BadChars));
        assert_eq!(
            PipelineName::new("x".repeat(64)),
            Err(InvalidName::TooLong { max: 63 })
        );
    }

    #[test]
    fn serde_round_trips_and_validates() {
        let v: VertexName = serde_json::from_str("\"in\"").unwrap();
        assert_eq!(v.as_str(), "in");
        assert!(serde_json::from_str::<VertexName>("\"Bad Name\"").is_err());
    }
}
