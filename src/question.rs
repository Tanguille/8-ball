use rmcp::schemars;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Question {
    /// Caller-chosen id, unique within one request. For a spec, the spec name.
    pub id: String,
    pub instructions: String,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Kind {
    Noul {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    Choice {
        /// option name -> description ("" = none). Order is kept.
        #[schemars(with = "std::collections::BTreeMap<String, String>")]
        criteria: indexmap::IndexMap<String, String>,
    },
    Score {
        /// level descriptions, lowest first, 2..=10
        criteria: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NoulCriteria {
    #[serde(rename = "true")]
    pub yes: String,
    #[serde(rename = "false")]
    pub no: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct QuestionError(pub String);

impl Kind {
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        match self {
            Self::Noul { .. } => vec!["yes".into(), "no".into()],
            Self::Choice { criteria } => criteria.keys().cloned().collect(),
            Self::Score { criteria } => {
                (0..criteria.len()).map(|index| index.to_string()).collect()
            }
        }
    }

    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Noul { .. } => "noul",
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
        }
    }
}

impl Question {
    pub fn validate(&self) -> Result<(), QuestionError> {
        if self.instructions.is_empty() {
            return Err(QuestionError("instructions must not be empty".into()));
        }

        match &self.kind {
            Kind::Choice { criteria } if criteria.len() < 2 => {
                Err(QuestionError("choice needs at least 2 options".into()))
            }
            Kind::Score { criteria } if !(2..=10).contains(&criteria.len()) => {
                Err(QuestionError("score needs 2 to 10 levels".into()))
            }
            _ => Ok(()),
        }
    }
}

pub fn validate_all(questions: &[Question]) -> Result<(), QuestionError> {
    if questions.is_empty() {
        return Err(QuestionError("no questions".into()));
    }
    let mut ids = std::collections::HashSet::new();
    for question in questions {
        question.validate()?;
        if !ids.insert(&question.id) {
            return Err(QuestionError(format!(
                "duplicate question id: {}",
                question.id
            )));
        }
    }

    Ok(())
}
