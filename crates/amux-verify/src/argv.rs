//! Splits a gate `cmd` string into argv without invoking a shell.
//!
//! POSIX-flavoured quoting: whitespace separates words, single/double
//! quotes group, backslash escapes the next character bare; inside double
//! quotes it escapes only `$`, backtick, `"` and `\` (POSIX rule) and stays
//! literal otherwise; inside single quotes everything is literal. Unquoted shell operators
//! are rejected: there is no shell at run time, so `&&` or `$VAR` would
//! reach the program as literal arguments — never what the author meant.
//! `*`, `?`, `~`, `=`, `#` are allowed and stay literal (no globbing, no
//! expansion, no comments).

/// Characters that would change command structure under a shell. Unquoted
/// occurrences are errors; quoted they are literal arguments.
const OPERATORS: &[char] = &['&', '|', ';', '<', '>', '$', '`', '(', ')'];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    UnterminatedQuote,
    TrailingBackslash,
    /// An unquoted shell operator (the offending token).
    ShellOperator(String),
}

impl std::fmt::Display for SplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SplitError::UnterminatedQuote => write!(f, "unterminated quote"),
            SplitError::TrailingBackslash => write!(f, "trailing backslash"),
            SplitError::ShellOperator(tok) => write!(
                f,
                "shell operators are not supported ({tok}); wrap the command in a script"
            ),
        }
    }
}

impl std::error::Error for SplitError {}

pub fn split(cmd: &str) -> Result<Vec<String>, SplitError> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = cmd.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => current.push(c),
                        None => return Err(SplitError::UnterminatedQuote),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => current.push(c),
                            Some(c) => {
                                current.push('\\');
                                current.push(c);
                            }
                            None => return Err(SplitError::UnterminatedQuote),
                        },
                        Some(c) => current.push(c),
                        None => return Err(SplitError::UnterminatedQuote),
                    }
                }
            }
            '\\' => match chars.next() {
                Some(c) => {
                    in_word = true;
                    current.push(c);
                }
                None => return Err(SplitError::TrailingBackslash),
            },
            c if OPERATORS.contains(&c) => {
                let token = if (c == '&' || c == '|') && chars.peek() == Some(&c) {
                    format!("{c}{c}")
                } else {
                    c.to_string()
                };
                return Err(SplitError::ShellOperator(token));
            }
            c => {
                in_word = true;
                current.push(c);
            }
        }
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain_words() {
        assert_eq!(
            split("cargo build --locked").unwrap(),
            vec!["cargo", "build", "--locked"]
        );
    }

    #[test]
    fn collapses_repeated_whitespace() {
        assert_eq!(split("a  \t b").unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn single_quotes_group_and_keep_backslashes() {
        assert_eq!(
            split("pytest -k 'not slow'").unwrap(),
            vec!["pytest", "-k", "not slow"]
        );
        assert_eq!(split(r"echo 'a\b'").unwrap(), vec!["echo", r"a\b"]);
    }

    #[test]
    fn double_quotes_group_and_unescape() {
        assert_eq!(split(r#"echo "a b""#).unwrap(), vec!["echo", "a b"]);
        assert_eq!(split(r#"echo "a\"b""#).unwrap(), vec!["echo", r#"a"b"#]);
    }

    #[test]
    fn bare_backslash_escapes_next_char() {
        assert_eq!(split(r"echo a\ b").unwrap(), vec!["echo", "a b"]);
    }

    #[test]
    fn empty_quotes_make_empty_arg() {
        assert_eq!(split("run ''").unwrap(), vec!["run", ""]);
    }

    #[test]
    fn quoted_operators_are_literal() {
        assert_eq!(split("grep '&&' src").unwrap(), vec!["grep", "&&", "src"]);
        assert_eq!(split(r#"echo "$HOME""#).unwrap(), vec!["echo", "$HOME"]);
    }

    #[test]
    fn glob_tilde_equals_hash_are_literal_words() {
        assert_eq!(
            split("pytest tests/* -x? ~/x FOO=bar #tag").unwrap(),
            vec!["pytest", "tests/*", "-x?", "~/x", "FOO=bar", "#tag"]
        );
    }

    #[test]
    fn unicode_survives() {
        assert_eq!(
            split("echo 'тест юникода'").unwrap(),
            vec!["echo", "тест юникода"]
        );
    }

    #[test]
    fn rejects_unquoted_operators() {
        for (cmd, tok) in [
            ("a && b", "&&"),
            ("a || b", "||"),
            ("a | b", "|"),
            ("a ; b", ";"),
            ("a > f", ">"),
            ("a < f", "<"),
            ("echo $HOME", "$"),
            ("echo `id`", "`"),
            ("(a)", "("),
            ("a)", ")"),
            ("a & b", "&"),
        ] {
            assert_eq!(
                split(cmd).unwrap_err(),
                SplitError::ShellOperator(tok.to_string()),
                "cmd: {cmd}"
            );
        }
    }

    #[test]
    fn rejects_unterminated_quotes_and_trailing_backslash() {
        assert_eq!(
            split("echo 'abc").unwrap_err(),
            SplitError::UnterminatedQuote
        );
        assert_eq!(
            split(r#"echo "abc"#).unwrap_err(),
            SplitError::UnterminatedQuote
        );
        assert_eq!(
            split(r"echo abc\").unwrap_err(),
            SplitError::TrailingBackslash
        );
    }

    #[test]
    fn empty_input_splits_to_no_words() {
        assert_eq!(split("").unwrap(), Vec::<String>::new());
        assert_eq!(split("   ").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn adjacent_segments_concatenate_into_one_word() {
        assert_eq!(split(r#"--flag="x y""#).unwrap(), vec!["--flag=x y"]);
        assert_eq!(split("a'b'c").unwrap(), vec!["abc"]);
        assert_eq!(split(r"'don'\''t'").unwrap(), vec!["don't"]);
    }

    #[test]
    fn double_quote_backslash_is_posix_accurate() {
        assert_eq!(split(r#"echo "a\nb""#).unwrap(), vec!["echo", r"a\nb"]);
        assert_eq!(split(r#"echo "a\$b""#).unwrap(), vec!["echo", "a$b"]);
        assert_eq!(split(r#"echo "a\\b""#).unwrap(), vec!["echo", r"a\b"]);
    }
}
