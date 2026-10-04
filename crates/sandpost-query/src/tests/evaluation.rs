use super::*;

#[test]
/// Verify collection inequality uses existential matching and headers retain duplicates.
fn recipient_any_not_equal_and_headers() {
    assert!(
        compile("to.address != \"absent@test.org\"")
            .unwrap()
            .evaluate(&facts())
    );
    assert!(
        compile("to.address != \"b@test.org\"")
            .unwrap()
            .evaluate(&facts())
    );
    assert!(
        compile("to.address == \"B@TEST.ORG\"")
            .unwrap()
            .evaluate(&facts())
    );
    assert!(
        compile("header[\"X-TAG\"] == \"green\"")
            .unwrap()
            .evaluate(&facts())
    );
}

#[test]
/// Verify wildcard evaluation remains exact on very large body text.
fn wildcard_handles_large_text_without_semantic_cutoff() {
    let mut large = facts();
    large.text = "x".repeat(1_000_123);
    assert!(compile("text matches \"*\"").unwrap().evaluate(&large));
}
