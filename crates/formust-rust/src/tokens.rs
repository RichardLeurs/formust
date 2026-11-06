//! Token-level identity, ignoring comments and whitespace: the hash in a `.frm.rs` header and the
//! token-equality check of the tests.

use std::str::FromStr;

use proc_macro2::{Delimiter, TokenStream, TokenTree};

/// A stable 32-bit FNV-1a hash of a token stream. Comments and whitespace don't change it, and
/// neither does the spacing of punctuation (rustfmt turns `a =-1` into `a = -1`, which lexes
/// differently). rustc reads CRLF as LF, so that doesn't change it either, even in a multi-line string.
pub fn hash_stream(stream: TokenStream) -> u32 {
    let mut hash = Fnv(0x811c_9dc5);
    feed(stream, &mut hash);
    hash.0
}

struct Fnv(u32);

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u32::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0193);
        }
    }
}

fn feed(stream: TokenStream, hash: &mut Fnv) {
    for tree in stream {
        match tree {
            TokenTree::Group(group) => {
                let (open, close): (&[u8], &[u8]) = match group.delimiter() {
                    Delimiter::Parenthesis => (b"(", b")"),
                    Delimiter::Brace => (b"{", b"}"),
                    Delimiter::Bracket => (b"[", b"]"),
                    Delimiter::None => (b"", b""),
                };
                hash.write(open);
                feed(group.stream(), hash);
                hash.write(close);
            }
            TokenTree::Ident(ident) => {
                hash.write(ident.to_string().as_bytes());
                hash.write(b" ");
            }
            TokenTree::Punct(punct) => {
                let mut buf = [0; 4];
                hash.write(punct.as_char().encode_utf8(&mut buf).as_bytes());
            }
            TokenTree::Literal(literal) => {
                hash.write(literal.to_string().replace("\r\n", "\n").as_bytes());
                hash.write(b" ");
            }
        }
    }
}

/// The token hash of source text, or `None` if it doesn't lex or is too deep for the importer.
/// Never panics.
pub fn token_hash(src: &str) -> Option<u32> {
    crate::import::on_big_stack(|| {
        let stream = TokenStream::from_str(src).ok()?;
        crate::import::check_limits(&stream).ok()?;
        let hash = hash_stream(stream);
        proc_macro2::extra::invalidate_current_thread_spans();
        Some(hash)
    })
    .flatten()
}

/// Whether two texts are the same tokens. `false` if either doesn't lex.
pub fn tokens_equal(a: &str, b: &str) -> bool {
    crate::import::on_big_stack(|| {
        let (Ok(a), Ok(b)) = (TokenStream::from_str(a), TokenStream::from_str(b)) else {
            return false;
        };
        if crate::import::check_limits(&a).is_err() || crate::import::check_limits(&b).is_err() {
            return false;
        }
        let same = same_tokens(a, b);
        proc_macro2::extra::invalidate_current_thread_spans();
        same
    })
    .unwrap_or(false)
}

fn same_tokens(a: TokenStream, b: TokenStream) -> bool {
    let (mut a, mut b) = (a.into_iter(), b.into_iter());
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) => {
                let same = match (x, y) {
                    (TokenTree::Group(x), TokenTree::Group(y)) => {
                        x.delimiter() == y.delimiter() && same_tokens(x.stream(), y.stream())
                    }
                    (TokenTree::Ident(x), TokenTree::Ident(y)) => x == y,
                    (TokenTree::Punct(x), TokenTree::Punct(y)) => x.as_char() == y.as_char(),
                    (TokenTree::Literal(x), TokenTree::Literal(y)) => {
                        x.to_string().replace("\r\n", "\n") == y.to_string().replace("\r\n", "\n")
                    }
                    _ => false,
                };
                if !same {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH_PIN: u32 = 0x88e2deab;

    #[test]
    fn comments_and_whitespace_do_not_change_the_hash() {
        let a = token_hash("fn f() { a.b(1, \"x\"); }").unwrap();
        let b = token_hash("fn f()   {\n // note\n a . b ( 1, /* c */ \"x\" ) ;\r\n}").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_changed_token_changes_the_hash() {
        let a = token_hash("a.b(1)").unwrap();
        assert_ne!(a, token_hash("a.b(2)").unwrap());
        assert_ne!(a, token_hash("a.c(1)").unwrap());
        assert_ne!(token_hash("{a}").unwrap(), token_hash("(a)").unwrap());
        assert_ne!(token_hash("a b").unwrap(), token_hash("ab").unwrap());
        assert_ne!(token_hash("a+b").unwrap(), token_hash("a-b").unwrap());
        // rustfmt's changes aren't conflicts.
        assert_eq!(token_hash("a =-1").unwrap(), token_hash("a = - 1").unwrap());
    }

    #[test]
    fn crlf_in_a_multiline_string_is_the_same_tokens() {
        assert_eq!(
            token_hash("\"a\nb\"").unwrap(),
            token_hash("\"a\r\nb\"").unwrap()
        );
        assert!(tokens_equal("\"a\nb\"", "\"a\r\nb\""));
    }

    #[test]
    fn the_hash_is_stable_across_versions() {
        // A change here invalidates the src-hash of every .frm.rs already written.
        assert_eq!(
            token_hash("{ ui.label(\"x\"); }"),
            Some(HASH_PIN),
            "{:x?}",
            token_hash("{ ui.label(\"x\"); }")
        );
    }

    #[test]
    fn a_lex_error_is_none_not_a_panic() {
        assert_eq!(token_hash("\"unterminated"), None);
        assert_eq!(token_hash("(("), None);
        assert!(!tokens_equal("((", "(("));
    }
}
