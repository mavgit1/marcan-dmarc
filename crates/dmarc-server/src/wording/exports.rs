//! What is said about a CSV export request the server refuses: a query
//! parameter the export does not read.

/// A query parameter `name` that the export `file` does not read, with the
/// ones it does (`accepted`, in order).
pub fn unknown_parameter(file: &str, name: &str, accepted: &[&str]) -> String {
    format!(
        "{file} has no parameter '{name}'. It takes {}; a domain or organization is named by its id.",
        accepted.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_parameter_and_the_accepted_ones() {
        assert_eq!(
            unknown_parameter("senders.csv", "domain", &["organization_id", "domain_id", "since"]),
            "senders.csv has no parameter 'domain'. It takes organization_id, domain_id, since; a domain or organization is named by its id."
        );
    }
}
