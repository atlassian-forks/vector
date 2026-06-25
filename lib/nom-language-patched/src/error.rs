use std::fmt;

use nom::{
  error::{ContextError, ErrorKind, FromExternalError, ParseError},
  ErrorConvert,
};

/// This error type accumulates errors and their position when backtracking
/// through a parse tree. With some post processing,
/// it can be used to display user friendly error messages
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerboseError<I> {
  /// List of errors accumulated by `VerboseError`, containing the affected
  /// part of input data, and some context
  pub errors: Vec<(I, VerboseErrorKind)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Error context for `VerboseError`
pub enum VerboseErrorKind {
  /// Static string added by the `context` function
  Context(&'static str),
  /// Indicates which character was expected by the `char` function
  Char(char),
  /// Error kind given by various nom parsers
  Nom(ErrorKind),
}

impl<I> ParseError<I> for VerboseError<I> {
  fn from_error_kind(input: I, kind: ErrorKind) -> Self {
    VerboseError {
      errors: vec![(input, VerboseErrorKind::Nom(kind))],
    }
  }

  fn append(input: I, kind: ErrorKind, mut other: Self) -> Self {
    other.errors.push((input, VerboseErrorKind::Nom(kind)));
    other
  }

  fn from_char(input: I, c: char) -> Self {
    VerboseError {
      errors: vec![(input, VerboseErrorKind::Char(c))],
    }
  }
}

impl<I> ContextError<I> for VerboseError<I> {
  fn add_context(input: I, ctx: &'static str, mut other: Self) -> Self {
    other.errors.push((input, VerboseErrorKind::Context(ctx)));
    other
  }
}

impl<I, E> FromExternalError<I, E> for VerboseError<I> {
  /// Create a new error from an input position and an external error
  fn from_external_error(input: I, kind: ErrorKind, _e: E) -> Self {
    Self::from_error_kind(input, kind)
  }
}

impl<I: fmt::Display> fmt::Display for VerboseError<I> {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(f, "Parse error:")?;
    for (input, error) in &self.errors {
      match error {
        VerboseErrorKind::Nom(e) => writeln!(f, "{:?} at: {}", e, input)?,
        VerboseErrorKind::Char(c) => writeln!(f, "expected '{}' at: {}", c, input)?,
        VerboseErrorKind::Context(s) => writeln!(f, "in section '{}', at: {}", s, input)?,
      }
    }

    Ok(())
  }
}

impl<I: fmt::Debug + fmt::Display> std::error::Error for VerboseError<I> {}

impl From<VerboseError<&[u8]>> for VerboseError<Vec<u8>> {
  fn from(value: VerboseError<&[u8]>) -> Self {
    VerboseError {
      errors: value
        .errors
        .into_iter()
        .map(|(i, e)| (i.to_owned(), e))
        .collect(),
    }
  }
}

impl From<VerboseError<&str>> for VerboseError<String> {
  fn from(value: VerboseError<&str>) -> Self {
    VerboseError {
      errors: value
        .errors
        .into_iter()
        .map(|(i, e)| (i.to_owned(), e))
        .collect(),
    }
  }
}

impl<I> ErrorConvert<VerboseError<I>> for VerboseError<(I, usize)> {
  fn convert(self) -> VerboseError<I> {
    VerboseError {
      errors: self.errors.into_iter().map(|(i, e)| (i.0, e)).collect(),
    }
  }
}

impl<I> ErrorConvert<VerboseError<(I, usize)>> for VerboseError<I> {
  fn convert(self) -> VerboseError<(I, usize)> {
    VerboseError {
      errors: self.errors.into_iter().map(|(i, e)| ((i, 0), e)).collect(),
    }
  }
}

/// Transforms a `VerboseError` into a trace with input position information
///
/// The errors contain references to input data that must come from `input`,
/// because nom calculates byte offsets between them
pub fn convert_error<I: core::ops::Deref<Target = str>>(input: I, e: VerboseError<I>) -> String {
  use nom::Offset;
  use std::fmt::Write;

  let mut result = String::new();

  for (i, (substring, kind)) in e.errors.iter().enumerate() {
    // `input.offset(substring)` is pointer arithmetic that assumes `substring`
    // points inside `input`. When that does not hold (e.g. the substring comes
    // from a different input buffer), the raw offset can be meaningless and
    // enormous. Clamp it to the input length so all downstream indexing stays in
    // bounds.
    let offset = input.offset(substring).min(input.len());

    if input.is_empty() {
      match kind {
        VerboseErrorKind::Char(c) => {
          write!(&mut result, "{}: expected '{}', got empty input\n\n", i, c)
        }
        VerboseErrorKind::Context(s) => write!(&mut result, "{}: in {}, got empty input\n\n", i, s),
        VerboseErrorKind::Nom(e) => write!(&mut result, "{}: in {:?}, got empty input\n\n", i, e),
      }
    } else {
      let prefix = &input.as_bytes()[..offset];

      // Count the number of newlines in the first `offset` bytes of input
      let line_number = prefix.iter().filter(|&&b| b == b'\n').count() + 1;

      // Find the line that includes the subslice:
      // Find the *last* newline before the substring starts
      let line_begin = prefix
        .iter()
        .rev()
        .position(|&b| b == b'\n')
        .map(|pos| offset - pos)
        .unwrap_or(0);

      // Find the full line after that newline
      let line = input[line_begin..]
        .lines()
        .next()
        .unwrap_or(&input[line_begin..])
        .trim_end();

      // The (1-indexed) column of the substring within the displayed line,
      // clamped to the line so it stays in range.
      let column_number = (offset.saturating_sub(line_begin) + 1).min(line.len() + 1);

      // Build the caret indicator explicitly (`column_number - 1` leading spaces
      // followed by `^`) rather than using a `{:>width$}` format specifier, which
      // panics for very large widths ("Formatting argument out of range").
      let caret = format!("{}^", " ".repeat(column_number.saturating_sub(1)));

      match kind {
        VerboseErrorKind::Char(c) => {
          if let Some(actual) = substring.chars().next() {
            write!(
              &mut result,
              "{i}: at line {line_number}:\n\
               {line}\n\
               {caret}\n\
               expected '{expected}', found {actual}\n\n",
              i = i,
              line_number = line_number,
              line = line,
              caret = caret,
              expected = c,
              actual = actual,
            )
          } else {
            write!(
              &mut result,
              "{i}: at line {line_number}:\n\
               {line}\n\
               {caret}\n\
               expected '{expected}', got end of input\n\n",
              i = i,
              line_number = line_number,
              line = line,
              caret = caret,
              expected = c,
            )
          }
        }
        VerboseErrorKind::Context(s) => write!(
          &mut result,
          "{i}: at line {line_number}, in {context}:\n\
             {line}\n\
             {caret}\n\n",
          i = i,
          line_number = line_number,
          context = s,
          line = line,
          caret = caret,
        ),
        VerboseErrorKind::Nom(e) => write!(
          &mut result,
          "{i}: at line {line_number}, in {nom_err:?}:\n\
             {line}\n\
             {caret}\n\n",
          i = i,
          line_number = line_number,
          nom_err = e,
          line = line,
          caret = caret,
        ),
      }
    }
    // Because `write!` to a `String` is infallible, this `unwrap` is fine.
    .unwrap();
  }

  result
}

#[test]
fn convert_error_panic() {
  use nom::character::complete::char;
  use nom::IResult;

  let input = "";

  let _result: IResult<_, _, VerboseError<&str>> = char('x')(input);
}

#[test]
fn issue_1027_convert_error_panic_nonempty() {
  use nom::character::complete::char;
  use nom::sequence::pair;
  use nom::Err;
  use nom::IResult;
  use nom::Parser;

  let input = "a";

  let result: IResult<_, _, VerboseError<&str>> = pair(char('a'), char('b')).parse(input);
  let err = match result.unwrap_err() {
    Err::Error(e) => e,
    _ => unreachable!(),
  };

  let msg = convert_error(input, err);
  assert_eq!(
    msg,
    "0: at line 1:\na\n ^\nexpected \'b\', got end of input\n\n"
  );
}

// A substring from a different allocation than `input` makes
// `input.offset(substring)` produce a meaningless, enormous offset.
// `convert_error` must clamp it instead of panicking.
#[test]
fn convert_error_handles_substring_from_other_allocation() {
  use nom::error::ErrorKind;

  let input = "first line\nsecond line\n";

  let other = String::from("an unrelated buffer in a different allocation");
  let substring: &str = &other[..];

  let err = VerboseError {
    errors: vec![(substring, VerboseErrorKind::Nom(ErrorKind::Tag))],
  };

  let msg = convert_error(input, err);

  assert!(msg.contains("in Tag"));
  assert!(!msg.is_empty());
}

// The caret column must stay within the displayed line for an ordinary,
// in-bounds error.
#[test]
fn convert_error_clamps_caret_column_to_line() {
  use nom::error::ErrorKind;

  let input = "abc\ndef";
  // Empty substring pointing one past the end of the input.
  let substring = &input[input.len()..];

  let err = VerboseError {
    errors: vec![(substring, VerboseErrorKind::Nom(ErrorKind::Eof))],
  };

  let msg = convert_error(input, err);

  // The caret line must not be padded beyond the displayed line length + 1.
  let caret_line = msg
    .lines()
    .find(|l| l.trim_start().starts_with('^'))
    .expect("expected a caret line");
  assert!(
    caret_line.len() <= "def".len() + 1,
    "caret column was not clamped: {caret_line:?}"
  );
}

// A long single line yields a large column number; building the caret manually
// (rather than via a `{:>width$}` format specifier) keeps this from panicking.
#[test]
fn convert_error_handles_large_column_without_panicking() {
  use nom::error::ErrorKind;

  // A single ~200k-char line followed by the text where parsing "fails".
  let input = "x".repeat(200_000) + "parsing fails here";
  let input_str = input.as_str();

  // An in-bounds substring far from the start of the line (column ~199_001).
  let error_location = &input_str[199_000..];

  let err = VerboseError {
    errors: vec![(error_location, VerboseErrorKind::Nom(ErrorKind::Tag))],
  };

  let msg = convert_error(input_str, err);

  assert!(msg.contains("in Tag"));
  // The caret is positioned at the (1-indexed) column: 199_000 spaces then '^'.
  let caret_line = msg
    .lines()
    .find(|l| l.trim_start().starts_with('^'))
    .expect("expected a caret line");
  assert_eq!(caret_line.len(), 199_001);
}

// `convert_error` must never index `input` out of bounds, for any substring
// offset or `VerboseErrorKind`, including on multi-byte UTF-8 input (so we never
// slice on a non-char-boundary).
#[test]
fn convert_error_never_panics_on_any_offset() {
  use nom::error::ErrorKind;

  let inputs = [
    "2026/06/25 04:27:04 http: TLS handshake error from 10.62.0.229:49835: EOF",
    "héllo wörld\nsécond lïne with ünïcode é",
    "",
    "\n\n\n",
    "single line no newline",
  ];

  for input in inputs {
    for off in 0..=input.len() {
      if !input.is_char_boundary(off) {
        continue;
      }
      let substring = &input[off..];
      for kind in [
        VerboseErrorKind::Nom(ErrorKind::Tag),
        VerboseErrorKind::Context("context"),
        VerboseErrorKind::Char('z'),
      ] {
        let err = VerboseError {
          errors: vec![(substring, kind)],
        };

        let _ = convert_error(input, err);
      }
    }
  }
}
