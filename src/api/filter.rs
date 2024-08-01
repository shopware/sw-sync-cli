//! Data structures to build criteria objects for the shopware API

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct Criteria {
    pub limit: u64,
    pub page: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<CriteriaFilter>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<CriteriaSorting>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub associations: BTreeMap<String, Criteria>,
}

impl Default for Criteria {
    fn default() -> Self {
        Self {
            limit: Self::MAX_LIMIT,
            page: 1,
            sort: vec![],
            filter: vec![],
            associations: BTreeMap::new(),
        }
    }
}

impl Criteria {
    /// Maximum limit accepted by the API server
    pub const MAX_LIMIT: u64 = 500;

    pub fn add_filter(&mut self, filter: CriteriaFilter) {
        self.filter.push(filter);
    }

    pub fn add_sorting(&mut self, sorting: CriteriaSorting) {
        self.sort.push(sorting);
    }

    pub fn add_association<S: Into<String>>(&mut self, association_path: S) -> &mut Self {
        let mut current = self;

        for part in association_path.into().split('.') {
            current = current
                .associations
                .entry(part.to_string())
                .or_insert_with(Criteria::default);
        }

        current
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CriteriaSorting {
    pub field: String,
    pub order: CriteriaSortingOrder,
}

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub enum CriteriaSortingOrder {
    #[serde(rename = "ASC")]
    Ascending,
    #[serde(rename = "DESC")]
    Descending,
}

/// See https://developer.shopware.com/docs/resources/references/core-reference/dal-reference/filters-reference.html
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CriteriaFilter {
    Equals {
        field: String,
        value: serde_json::Value,
    },
    EqualsAny {
        field: String,
        value: Vec<serde_json::Value>,
    },
    Contains {
        field: String,
        value: serde_json::Value,
    },
    Range {
        field: String,
        parameters: RangeParameters,
    },
    Not {
        /// operator used WITHIN the not filter (between all queries)
        operator: LogicOperator,
        queries: Vec<CriteriaFilter>,
    },
    Multi {
        /// operator used WITHIN the multi filter (between all queries)
        operator: LogicOperator,
        queries: Vec<CriteriaFilter>,
    },
    Prefix {
        field: String,
        value: serde_json::Value,
    },
    Suffix {
        field: String,
        value: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogicOperator {
    And,
    Or,
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct RangeParameters {
    /// greater than equals
    #[serde(skip_serializing_if = "Option::is_none")]
    gte: Option<serde_json::Value>,
    /// less than equals
    #[serde(skip_serializing_if = "Option::is_none")]
    lte: Option<serde_json::Value>,
    /// greater than
    #[serde(skip_serializing_if = "Option::is_none")]
    gt: Option<serde_json::Value>,
    /// less than
    #[serde(skip_serializing_if = "Option::is_none")]
    lt: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct EmptyObject {
    // no fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn criteria_serialize_association() {
        let mut criteria = Criteria {
            limit: 10,
            page: 2,
            ..Default::default()
        };
        criteria.add_association("manufacturer");
        criteria.add_association("cover.media");

        let json = serde_json::to_string_pretty(&criteria).unwrap();
        assert_eq!(
            json,
            r#"{
  "limit": 10,
  "page": 2,
  "associations": {
    "cover": {
      "limit": 500,
      "page": 1,
      "associations": {
        "media": {
          "limit": 500,
          "page": 1
        }
      }
    },
    "manufacturer": {
      "limit": 500,
      "page": 1
    }
  }
}"#
        );
    }

    #[test]
    fn criteria_serialize_sorting() {
        let mut criteria = Criteria {
            limit: 10,
            page: 2,
            ..Default::default()
        };
        criteria.add_sorting(CriteriaSorting {
            field: "manufacturerId".to_string(),
            order: CriteriaSortingOrder::Descending,
        });

        let json = serde_json::to_string_pretty(&criteria).unwrap();
        assert_eq!(
            json,
            r#"{
  "limit": 10,
  "page": 2,
  "sort": [
    {
      "field": "manufacturerId",
      "order": "DESC"
    }
  ]
}"#
        );
    }

    #[test]
    fn criteria_serialize_filter() {
        let mut criteria = Criteria {
            limit: 10,
            page: 2,
            ..Default::default()
        };
        criteria.add_filter(CriteriaFilter::Equals {
            field: "parentId".to_string(),
            value: serde_json::Value::Null,
        });
        criteria.add_filter(CriteriaFilter::Not {
            operator: LogicOperator::And,
            queries: vec![
                CriteriaFilter::Contains {
                    field: "name".to_string(),
                    value: json!("shopware"),
                },
                CriteriaFilter::Range {
                    field: "stock".to_string(),
                    parameters: RangeParameters {
                        gte: Some(json!(20)),
                        lte: Some(json!(30)),
                        ..Default::default()
                    },
                },
            ],
        });

        let json = serde_json::to_string_pretty(&criteria).unwrap();
        assert_eq!(
            json,
            r#"{
  "limit": 10,
  "page": 2,
  "filter": [
    {
      "type": "equals",
      "field": "parentId",
      "value": null
    },
    {
      "type": "not",
      "operator": "and",
      "queries": [
        {
          "type": "contains",
          "field": "name",
          "value": "shopware"
        },
        {
          "type": "range",
          "field": "stock",
          "parameters": {
            "gte": 20,
            "lte": 30
          }
        }
      ]
    }
  ]
}"#
        );
    }
}
