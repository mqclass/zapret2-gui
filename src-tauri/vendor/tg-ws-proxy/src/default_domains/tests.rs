use super::*;

#[test]
fn parse_domain_list_skips_blank_lines_and_comments() {
    let domains = parse_domain_list("# header\nvirkgj.com\n\n# comment\nvmmzovy.com\n");

    assert_eq!(domains.len(), 2);
    assert!(domains.iter().all(|domain| domain.ends_with(".co.uk")));
}

#[test]
fn parse_domain_list_drops_entries_that_do_not_decode() {
    // A line that is not a `.com` entry cannot be deobfuscated and must be
    // skipped rather than passed through as a bogus domain.
    let domains = parse_domain_list("virkgj.com\nnot-encoded.org\nnocomhere\n");

    assert_eq!(domains, vec!["pclead.co.uk"]);
}

#[test]
fn fallback_domains_are_all_decodable() {
    // The built-in list is what users fall back to when GitHub is unreachable,
    // so every entry has to survive deobfuscation.
    let domains = fallback_domains();

    assert_eq!(domains.len(), FALLBACK_ENCODED.len());
    assert!(domains.iter().all(|domain| domain.ends_with(".co.uk")));
}
