//! A `Progress`'s stage plan: the ordered, weighted list of stages one
//! command will run through (SOPACK-1.0-PLAN.md §3.4's per-command table),
//! declared up front so an overall percentage exists even before the first
//! stage's `total` is known.

/// One planned stage. `weight` is relative — only its share of the sum of
/// every stage's weight in the plan matters, so `[("tokenise", 1.0),
/// ("embed", 4.0), ("write", 0.5)]` and `[("tokenise", 2.0), ("embed",
/// 8.0), ("write", 1.0)]` behave identically.
#[derive(Debug, Clone)]
pub struct StageWeight {
    pub name: &'static str,
    pub weight: f64,
}

impl StageWeight {
    pub fn new(name: &'static str, weight: f64) -> Self {
        StageWeight { name, weight }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_with_given_fields() {
        let s = StageWeight::new("embed", 4.0);
        assert_eq!(s.name, "embed");
        assert_eq!(s.weight, 4.0);
    }
}
