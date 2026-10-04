//! Whole-value wildcard matching over Unicode scalar values.

// ponytail: greedy matching is O(text * pattern) in the worst case; compile
// reusable patterns if measured policy workloads justify it. Never truncate a
// match, because changing a result also changes the meaning of `not`.
/// Match a complete string against `*` and `?` using Unicode scalar values.
pub(crate) fn glob_matches(text: &str, pattern: &str, basic_latin_case_insensitive: bool) -> bool {
    let pattern_characters: Vec<char> = pattern.chars().collect();
    if !pattern_characters.is_empty()
        && pattern_characters.iter().all(|character| *character == '*')
    {
        return true;
    }
    let (mut text_offset, mut pattern_index, mut last_star_index, mut retry_offset) =
        (0, 0, None, 0usize);
    while text_offset < text.len() {
        let text_character = text[text_offset..].chars().next().unwrap();
        if pattern_index < pattern_characters.len() && pattern_characters[pattern_index] == '*' {
            last_star_index = Some(pattern_index);
            pattern_index += 1;
            retry_offset = text_offset;
        } else if pattern_index < pattern_characters.len()
            && (pattern_characters[pattern_index] == '?'
                || if basic_latin_case_insensitive {
                    pattern_characters[pattern_index].eq_ignore_ascii_case(&text_character)
                } else {
                    pattern_characters[pattern_index] == text_character
                })
        {
            text_offset += text_character.len_utf8();
            pattern_index += 1;
        } else if let Some(star_index) = last_star_index {
            if retry_offset >= text.len() {
                return false;
            }
            retry_offset += text[retry_offset..].chars().next().unwrap().len_utf8();
            text_offset = retry_offset;
            pattern_index = star_index + 1;
        } else {
            return false;
        }
    }
    pattern_characters[pattern_index..]
        .iter()
        .all(|character| *character == '*')
}

#[cfg(test)]
mod tests {
    use super::glob_matches;

    /// Compute glob matching with an independent dynamic-programming test oracle.
    fn reference_glob_matches(text: &str, pattern: &str, ignore_basic_latin_case: bool) -> bool {
        let text_characters: Vec<char> = text.chars().collect();
        let mut previous_row = vec![false; text_characters.len() + 1];
        previous_row[0] = true;
        for pattern_character in pattern.chars() {
            let mut current_row = vec![false; text_characters.len() + 1];
            current_row[0] = pattern_character == '*' && previous_row[0];
            for (character_index, text_character) in text_characters.iter().enumerate() {
                current_row[character_index + 1] = if pattern_character == '*' {
                    previous_row[character_index + 1] || current_row[character_index]
                } else {
                    previous_row[character_index]
                        && (pattern_character == '?'
                            || if ignore_basic_latin_case {
                                pattern_character.eq_ignore_ascii_case(text_character)
                            } else {
                                pattern_character == *text_character
                            })
                };
            }
            previous_row = current_row;
        }
        previous_row[text_characters.len()]
    }

    /// Generate every string over an alphabet up to the requested character length.
    fn generate_strings_from_alphabet(alphabet: &[char], maximum_length: usize) -> Vec<String> {
        let mut values = vec![String::new()];
        let mut frontier = values.clone();
        for _ in 0..maximum_length {
            frontier = frontier
                .iter()
                .flat_map(|prefix| {
                    alphabet
                        .iter()
                        .map(move |character| format!("{prefix}{character}"))
                })
                .collect();
            values.extend(frontier.iter().cloned());
        }
        values
    }

    /// Compare the greedy matcher with an independent oracle over short Unicode cases.
    #[test]
    fn exhaustive_short_globs_agree_with_independent_oracle() {
        let text_samples = generate_strings_from_alphabet(&['a', 'A', 'é'], 3);
        let pattern_samples = generate_strings_from_alphabet(&['a', 'A', 'é', '*', '?'], 4);
        for text in &text_samples {
            for pattern in &pattern_samples {
                for ignore_basic_latin_case in [false, true] {
                    assert_eq!(
                        glob_matches(text, pattern, ignore_basic_latin_case),
                        reference_glob_matches(text, pattern, ignore_basic_latin_case),
                        "text={text:?}, pattern={pattern:?}, ignore_basic_latin_case={ignore_basic_latin_case}"
                    );
                }
            }
        }
    }

    /// Cover retry behavior, Unicode scalars, and literal bracket characters.
    /// Cover wildcard retries, Unicode scalars, and literal bracket characters.
    #[test]
    fn retries_unicode_and_literal_wildcard_characters() {
        for (text, pattern) in [
            ("aaaaab", "*aaab"),
            ("abacabad", "*aba*bad"),
            ("*xa", "*a"),
            ("café🙂", "caf??"),
            ("e\u{301}", "??"),
            ("[abc]", "[abc]"),
        ] {
            assert!(glob_matches(text, pattern, false));
        }
        for (text, pattern) in [
            ("abc", "[abc]"),
            ("é", "É"),
            ("e\u{301}", "?"),
            ("aaaaab", "*aaac"),
        ] {
            assert!(!glob_matches(text, pattern, true));
        }
    }

    /// Ensure long text remains matched exactly after repeated wildcard retries.
    /// Ensure repeated wildcard retries still produce an exact result on long input.
    #[test]
    fn long_text_is_matched_exactly_after_many_retries() {
        let text = "x".repeat(1_000_123) + "ab";
        assert!(glob_matches(&text, "*ab", false));
        assert!(!glob_matches(&text, "*ac", false));
    }
}
