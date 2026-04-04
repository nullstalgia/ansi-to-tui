use crate::code::AnsiCode;
use nom::{
    branch::alt,
    bytes::complete::*,
    character::{complete::*, is_alphabetic},
    combinator::{map, map_res, opt, recognize, value},
    error::{self, Error, ErrorKind, FromExternalError},
    multi::*,
    sequence::{delimited, preceded, terminated, tuple},
    IResult, Parser,
};
use std::{borrow::Cow, char::REPLACEMENT_CHARACTER, str::FromStr};
use tui::{
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
/// When using lossy conversion, specify whether to replace invalid bytes with the
/// replacement character (�), or show escape their values to be printed.
///
/// An optional [Style] can be given for any replaced characters.
pub enum LossyFlavor {
    /// Invalid UTF-8 sequences will be replaced with
    /// [`U+FFFD REPLACEMENT CHARACTER`][U+FFFD], which looks like this: �
    ReplacementChar(Option<Style>),
    /// Invalid UTF-8 sequences will be escaped with `\xFF` notation.
    EscapedBytes(Option<Style>),
    /// Invalid UTF-8 sequences will be omitted entirely.
    Omitted,
}

impl LossyFlavor {
    /// Replace any invalid UTF-8 sequences with �, using the text's [Style].
    pub fn replacement_char() -> Self {
        Self::ReplacementChar(None)
    }
    /// Replace any invalid UTF-8 sequences with � and the given [Style].
    pub fn replacement_char_styled(style: Style) -> Self {
        Self::ReplacementChar(Some(style))
    }
    /// Escape invalid UTF-8 bytes using `\xFF` notation, with the text's [Style].
    pub fn escaped_bytes() -> Self {
        Self::EscapedBytes(None)
    }
    /// Escape invalid UTF-8 bytes using `\xFF` notation and the given [Style].
    pub fn escaped_bytes_styled(style: Style) -> Self {
        Self::EscapedBytes(Some(style))
    }
    /// Invalid UTF-8 sequences will be omitted entirely.
    pub fn omitted() -> Self {
        Self::Omitted
    }
    /// Get the [Style] of the flavor, if specified.
    pub fn style(&self) -> Option<Style> {
        match self {
            LossyFlavor::ReplacementChar(style) => *style,
            LossyFlavor::EscapedBytes(style) => *style,
            LossyFlavor::Omitted => None,
        }
    }
}

struct ValidAndReplacementSpans<'a> {
    valid: Span<'a>,
    replacement: Option<Span<'static>>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ValueOrClear<T> {
    Value(T),
    ClearLine,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ColorType {
    /// Eight Bit color
    EightBit,
    /// 24-bit color or true color
    TrueColor,
}

#[derive(Debug, Clone, PartialEq)]
struct AnsiItem {
    code: AnsiCode,
    color: Option<Color>,
}

#[derive(Debug, Clone, PartialEq)]
struct AnsiStates {
    pub items: smallvec::SmallVec<[AnsiItem; 2]>,
    pub style: Style,
}

impl From<AnsiStates> for tui::style::Style {
    fn from(states: AnsiStates) -> Self {
        let mut style = states.style;
        if states.items.is_empty() {
            // https://github.com/uttarayan21/ansi-to-tui/issues/40
            // [m should be treated as a reset as well
            style = Style::reset();
        }
        for item in states.items {
            match item.code {
                AnsiCode::Reset => style = Style::reset(),
                AnsiCode::Bold => style = style.add_modifier(Modifier::BOLD),
                AnsiCode::Faint => style = style.add_modifier(Modifier::DIM),
                AnsiCode::Normal => {
                    style = style.remove_modifier(Modifier::BOLD | Modifier::DIM);
                }
                AnsiCode::Italic => style = style.add_modifier(Modifier::ITALIC),
                AnsiCode::NotItalic => style = style.remove_modifier(Modifier::ITALIC),
                AnsiCode::Underline => style = style.add_modifier(Modifier::UNDERLINED),
                AnsiCode::UnderlineOff => style = style.remove_modifier(Modifier::UNDERLINED),
                AnsiCode::SlowBlink => style = style.add_modifier(Modifier::SLOW_BLINK),
                AnsiCode::RapidBlink => style = style.add_modifier(Modifier::RAPID_BLINK),
                AnsiCode::BlinkOff => {
                    style = style.remove_modifier(Modifier::SLOW_BLINK | Modifier::RAPID_BLINK)
                }
                AnsiCode::Reverse => style = style.add_modifier(Modifier::REVERSED),
                AnsiCode::Conceal => style = style.add_modifier(Modifier::HIDDEN),
                AnsiCode::Reveal => style = style.remove_modifier(Modifier::HIDDEN),
                AnsiCode::CrossedOut => style = style.add_modifier(Modifier::CROSSED_OUT),
                AnsiCode::CrossedOutOff => style = style.remove_modifier(Modifier::CROSSED_OUT),
                AnsiCode::DefaultForegroundColor => style = style.fg(Color::Reset),
                AnsiCode::DefaultBackgroundColor => style = style.bg(Color::Reset),
                AnsiCode::SetForegroundColor => {
                    if let Some(color) = item.color {
                        style = style.fg(color)
                    }
                }
                AnsiCode::SetBackgroundColor => {
                    if let Some(color) = item.color {
                        style = style.bg(color)
                    }
                }
                AnsiCode::ForegroundColor(color) => style = style.fg(color),
                AnsiCode::BackgroundColor(color) => style = style.bg(color),
                _ => (),
            }
        }
        style
    }
}

pub(crate) fn text<'a>(
    mut s: &'a [u8],
    line_ending: &str,
    lossy: Option<LossyFlavor>,
) -> IResult<&'a [u8], Text<'static>> {
    let mut lines = Vec::new();
    let mut last = Style::new();
    while let Ok((_s, (line, style))) = line(last, Some(line_ending), lossy)(s) {
        lines.push(line);
        last = style;
        s = _s;
        if s.is_empty() {
            break;
        }
    }
    Ok((s, Text::from(lines)))
}

#[cfg(feature = "zero-copy")]
pub(crate) fn text_fast<'a>(
    mut s: &'a [u8],
    line_ending: &str,
    lossy: Option<LossyFlavor>,
) -> IResult<&'a [u8], Text<'a>> {
    let mut lines = Vec::new();
    let mut last = Style::new();
    while let Ok((_s, (line, style, _spans_cleared))) = line_fast(last, Some(line_ending), lossy)(s)
    {
        lines.push(line);
        last = style;
        s = _s;
        if s.is_empty() {
            break;
        }
    }
    Ok((s, Text::from(lines)))
}

pub(crate) fn line(
    style: Style,
    line_ending: Option<&'_ str>,
    lossy: Option<LossyFlavor>,
) -> impl Fn(&[u8]) -> IResult<&[u8], (Line<'static>, Style)> + '_ {
    move |s: &[u8]| -> IResult<&[u8], (Line<'static>, Style)> {
        let (s, mut text) = take_until_line_ending(line_ending, true)(s)?;

        let mut spans = Vec::new();
        let mut last = style;
        while let Ok((s, span)) = span(last, line_ending, lossy)(text) {
            match span {
                ValueOrClear::ClearLine => spans.clear(),
                ValueOrClear::Value(ValidAndReplacementSpans { valid, replacement }) => {
                    // Since reset now tracks seperately we can skip the reset check
                    last = last.patch(valid.style);

                    if !valid.content.is_empty() {
                        spans.push(valid);
                    }

                    if let Some(replacement) = replacement {
                        spans.push(replacement);
                    }
                }
            }

            text = s;
            if text.is_empty() {
                break;
            }
        }

        Ok((s, (Line::from(spans), last)))
    }
}

#[cfg(feature = "zero-copy")]
pub(crate) fn line_fast(
    style: Style,
    line_ending: Option<&'_ str>,
    lossy: Option<LossyFlavor>,
) -> impl Fn(&[u8]) -> IResult<&[u8], (Line<'_>, Style, Option<(usize, Style)>)> + '_ {
    move |s: &[u8]| -> IResult<&[u8], (Line<'_>, Style, Option<(usize, Style)>)> {
        let (s, mut text) = take_until_line_ending(line_ending, true)(s)?;
        let original_text_len = text.len();
        let mut spans = Vec::new();
        let mut spans_cleared_at = None;
        let mut last = style;
        while let Ok((s, span_and_replacement)) = span_fast(last, line_ending, lossy)(text) {
            match span_and_replacement {
                ValueOrClear::ClearLine => {
                    spans.clear();
                    spans_cleared_at = Some((original_text_len - s.len(), last));
                }
                ValueOrClear::Value(ValidAndReplacementSpans { valid, replacement }) => {
                    last = last.patch(valid.style);
                    // If the spans is empty then it might be possible that the style changes
                    // but there is no text change
                    if !valid.content.is_empty() {
                        spans.push(valid);
                    }

                    if let Some(replacement) = replacement {
                        spans.push(replacement);
                    }
                }
            }

            text = s;
            if text.is_empty() {
                break;
            }
        }

        Ok((s, (Line::from(spans), last, spans_cleared_at)))
    }
}

#[allow(clippy::type_complexity)]
fn span(
    last: Style,
    line_ending: Option<&'_ str>,
    lossy: Option<LossyFlavor>,
) -> impl Fn(
    &[u8],
) -> IResult<
    &[u8],
    ValueOrClear<ValidAndReplacementSpans<'static>>,
    nom::error::Error<&[u8]>,
> + '_ {
    move |s: &[u8]| -> IResult<&[u8], ValueOrClear<ValidAndReplacementSpans<'static>>> {
        // If s starts with line ending, return immediately
        // if let Some(le) = line_ending {
        //     let le_bytes = le.as_bytes();
        //     if !le_bytes.is_empty() && s.starts_with(le_bytes) {
        //         // nom: return as-is, empty span (for line ending-separators)
        //         return Ok((
        //             &s[le_bytes.len()..],
        //             ValueOrClear::Value(ValidAndReplacementSpans {
        //                 valid: Span::styled(le.to_owned(), last),
        //                 replacement: None,
        //             }),
        //         ));
        //     }
        // }

        let mut last = last;
        let (s, style) = opt(style(last))(s)?;

        match style.flatten() {
            Some(ValueOrClear::ClearLine) => {
                return Ok((s, ValueOrClear::ClearLine));
            }
            Some(ValueOrClear::Value(style)) => {
                last = last.patch(style);
            }
            None => (),
        }

        if let Some(flavor) = lossy {
            let (rest, bytes) = take_until_next_esc_or_line_ending(line_ending)(s)?;

            #[cfg(not(feature = "simd"))]
            let res = std::str::from_utf8(bytes);

            #[cfg(feature = "simd")]
            let res = simdutf8::compat::from_utf8(bytes);

            match res {
                Ok(txt) => Ok((
                    rest,
                    ValueOrClear::Value(ValidAndReplacementSpans {
                        valid: Span::styled(txt.to_owned(), last),
                        replacement: None,
                    }),
                )),
                Err(e) => {
                    let (valid, after_valid) = s.split_at(e.valid_up_to());

                    let valid = if valid.is_empty() {
                        ""
                    } else {
                        // SAFETY: simdutf8::compat's docs state that it's Utf8Error is analogous
                        // to stdlib's of which says `valid_up_to`:
                        // > Returns the index in the given string up to which __valid UTF-8 was verified.__
                        unsafe { std::str::from_utf8_unchecked(valid) }
                    };

                    let replacement_style = flavor.style().unwrap_or(last);

                    let invalid = match e.error_len() {
                        // Input ended unexpectedly, consume as if it's malformed.
                        None => after_valid,
                        Some(invalid) => &after_valid[..invalid],
                    };

                    let replacement = {
                        match flavor {
                            LossyFlavor::ReplacementChar(_) => {
                                let cow = Cow::Borrowed("\u{FFFD}");
                                Some(Span::styled(cow, replacement_style))
                            }
                            LossyFlavor::EscapedBytes(_) => {
                                let cow = Cow::Owned(
                                    invalid.iter().map(|b| format!(r"\x{b:02X}")).collect(),
                                );
                                Some(Span::styled(cow, replacement_style))
                            }
                            LossyFlavor::Omitted => None,
                        }
                    };

                    Ok((
                        &s[valid.len() + invalid.len()..],
                        ValueOrClear::Value(ValidAndReplacementSpans {
                            valid: Span::styled(valid.to_owned(), last),
                            replacement,
                        }),
                    ))
                }
            }
        } else {
            #[cfg(feature = "simd")]
            let (s, text) = map_res(take_until_next_esc_or_line_ending(line_ending), |t| {
                simdutf8::basic::from_utf8(t)
            })(s)?;

            #[cfg(not(feature = "simd"))]
            let (s, text) = map_res(take_until_next_esc_or_line_ending(line_ending), |t| {
                std::str::from_utf8(t)
            })(s)?;

            Ok((
                s,
                ValueOrClear::Value(ValidAndReplacementSpans {
                    valid: Span::styled(text.to_owned(), last),
                    replacement: None,
                }),
            ))
        }
    }
}

#[cfg(feature = "zero-copy")]
#[allow(clippy::type_complexity)]
fn span_fast(
    last: Style,
    line_ending: Option<&'_ str>,
    lossy: Option<LossyFlavor>,
) -> impl Fn(&[u8]) -> IResult<&[u8], ValueOrClear<ValidAndReplacementSpans>, nom::error::Error<&[u8]>>
       + '_ {
    move |s: &[u8]| -> IResult<&[u8], ValueOrClear<ValidAndReplacementSpans>> {
        // If s starts with line ending, return immediately
        // if let Some(le) = line_ending {
        //     let le_bytes = le.as_bytes();
        //     if !le_bytes.is_empty() && s.starts_with(le_bytes) {
        //         // nom: return as-is, empty span (for line ending-separators)
        //         return Ok((
        //             &s[le_bytes.len()..],
        //             ValueOrClear::Value(ValidAndReplacementSpans {
        //                 valid: Span::styled(le.to_owned(), last),
        //                 replacement: None,
        //             }),
        //         ));
        //     }
        // }

        let mut last = last;
        let (s, style) = opt(style(last))(s)?;

        match style.flatten() {
            Some(ValueOrClear::ClearLine) => {
                return Ok((s, ValueOrClear::ClearLine));
            }
            Some(ValueOrClear::Value(style)) => {
                last = last.patch(style);
            }
            None => (),
        }

        let (s, text, replacement) = if let Some(flavor) = lossy {
            let (rest, bytes) = take_until_next_esc_or_line_ending(line_ending)(s)?;

            #[cfg(not(feature = "simd"))]
            let res = std::str::from_utf8(bytes);

            #[cfg(feature = "simd")]
            let res = simdutf8::compat::from_utf8(bytes);

            match res {
                Ok(txt) => (rest, txt, None),
                Err(e) => {
                    let (valid, after_valid) = s.split_at(e.valid_up_to());

                    let valid = if valid.is_empty() {
                        ""
                    } else {
                        // SAFETY: simdutf8::compat's docs state that it's Utf8Error is analogous
                        // to stdlib's of which says `valid_up_to`:
                        // > Returns the index in the given string up to which __valid UTF-8 was verified.__
                        unsafe { std::str::from_utf8_unchecked(valid) }
                    };

                    let replacement_style = flavor.style().unwrap_or(last);

                    let invalid = match e.error_len() {
                        // Input ended unexpectedly, consume as if it's malformed.
                        None => after_valid,
                        Some(invalid) => &after_valid[..invalid],
                    };

                    let replacement = {
                        match flavor {
                            LossyFlavor::ReplacementChar(_) => {
                                let cow = Cow::Borrowed("\u{FFFD}");
                                Some(Span::styled(cow, replacement_style))
                            }
                            LossyFlavor::EscapedBytes(_) => {
                                let cow = Cow::Owned(
                                    invalid.iter().map(|b| format!(r"\x{b:02X}")).collect(),
                                );
                                Some(Span::styled(cow, replacement_style))
                            }
                            LossyFlavor::Omitted => None,
                        }
                    };

                    (&s[valid.len() + invalid.len()..], valid, replacement)
                }
            }
        } else {
            #[cfg(feature = "simd")]
            let (s, text) = map_res(take_until_next_esc_or_line_ending(line_ending), |t| {
                simdutf8::basic::from_utf8(t)
            })(s)?;

            #[cfg(not(feature = "simd"))]
            let (s, text) = map_res(take_until_next_esc_or_line_ending(line_ending), |t| {
                std::str::from_utf8(t)
            })(s)?;

            (s, text, None)
        };

        let valid = Span::styled(text, last);

        Ok((
            s,
            ValueOrClear::Value(ValidAndReplacementSpans { valid, replacement }),
        ))
    }
}

#[allow(clippy::type_complexity)]
fn style(
    style: Style,
) -> impl Fn(&[u8]) -> IResult<&[u8], Option<ValueOrClear<Style>>, nom::error::Error<&[u8]>> {
    move |s: &[u8]| -> IResult<&[u8], Option<ValueOrClear<Style>>> {
        let (s, r) = match opt(ansi_sgr_code)(s)? {
            (s, Some(r)) => (
                s,
                Some(ValueOrClear::Value(Style::from(AnsiStates {
                    style,
                    items: r,
                }))),
            ),
            (s, None) => match opt(clear_line_code)(s)? {
                (s, Some(r)) => (s, r),
                (s, None) => {
                    let (s, _) = any_escape_sequence(s)?;
                    (s, None)
                }
            },
            // (s, None) => {
            //     // if no style found, check for a Erase Line command
            //     let (s, clear) = opt(clear_line_code)(s)?;
            //     if let Some(Some(clear)) = clear {
            //         return Ok((s, Some(clear)));
            //     }
            // }
        };
        Ok((s, r))
    }
}

/// Parse a Clear Line code
fn clear_line_code(
    s: &[u8],
) -> IResult<&[u8], Option<ValueOrClear<Style>>, nom::error::Error<&[u8]>> {
    // Recognizes clear line escape codes: \x1b[K, \x1b[0K, \x1b[1K, \x1b[2K] with potentially-present \r prefix.
    // See https://vt100.net/docs/vt510-rm/EL.html for details about the "Erase in Line" (EL/K) sequences.

    // Patterns to match:
    // 1. \x1b[K (ESC [ K)         - with preceding \r, clears line; alone, no effect
    // 2. \x1b[0K (ESC [ 0 K)      - with preceding \r, clears line; alone, no effect
    // 3. \x1b[1K (ESC [ 1 K)      - clears from start of line to cursor; i.e., with no \r, clears left
    // 4. \x1b[2K (ESC [ 2 K)      - clears entire line, always
    //
    // noms: [optional \r][ESC][ [ ('K' | '0K' | '1K' | '2K') ]

    use nom::character::complete::char as cchar;

    // Helper for leading '\r', returns the rest
    let cr = opt(cchar('\r'));

    // Matches ESC [
    let esc_bracket = tuple((cchar('\x1b'), cchar('[')));

    // Matches the possible code after ESC[
    // "K", or "0K", or "1K", or "2K"
    fn el_code(input: &[u8]) -> IResult<&[u8], u8, nom::error::Error<&[u8]>> {
        // Only match 1K, 2K, 0K, or K (must be ASCII)
        alt((
            map(tag("2K"), |_| 2),
            map(tag("1K"), |_| 1),
            map(tag("0K"), |_| 0),
            map(tag("K"), |_| 0),
        ))(input)
    }

    // Full matcher: optional \r, then ESC [, then one of the codes.
    let (rest, (cr_present, _, code)) = match tuple((cr, esc_bracket, el_code))(s) {
        Ok(v) => v,
        Err(e) => return Err(e), // Did not match, nothing consumed from input.
    };

    // Determine if this sequence actually has an effect on the current line.
    let should_clear = match (code, cr_present.is_some()) {
        (2, _) => true,     // 2K always clears the line
        (1, false) => true, // 1K clears from start to cursor = clear left
        (0, true) => true,  // K or 0K with \r clears line
        _ => false,
    };

    if should_clear {
        Ok((rest, Some(ValueOrClear::ClearLine)))
    } else {
        Ok((rest, None))
    }
}

/// A complete ANSI SGR code
fn ansi_sgr_code(
    s: &[u8],
) -> IResult<&[u8], smallvec::SmallVec<[AnsiItem; 2]>, nom::error::Error<&[u8]>> {
    delimited(
        tag("\x1b["),
        fold_many0(ansi_sgr_item, smallvec::SmallVec::new, |mut items, item| {
            items.push(item);
            items
        }),
        char('m'),
    )(s)
}

fn any_escape_sequence(s: &[u8]) -> IResult<&[u8], Option<&[u8]>> {
    // Attempt to consume most escape codes, including a single escape char.
    //
    // Most escape codes begin with ESC[ and are terminated by an alphabetic character,
    // but OSC codes begin with ESC] and are terminated by an ascii bell (\x07)
    // and a truncated/invalid code may just be a standalone ESC or not be terminated.
    //
    // We should try to consume as much of it as possible to match behavior of most terminals;
    // where we fail at that we should at least consume the escape char to avoid infinitely looping

    let (input, garbage) = preceded(
        char('\x1b'),
        opt(alt((
            delimited(char('['), take_till(is_alphabetic), opt(take(1u8))),
            delimited(char(']'), take_till(|c| c == b'\x07'), opt(take(1u8))),
        ))),
    )(s)?;
    Ok((input, garbage))
}

/// An ANSI SGR attribute
fn ansi_sgr_item(s: &[u8]) -> IResult<&[u8], AnsiItem> {
    let (s, c) = u8(s)?;
    let code = AnsiCode::from(c);
    let (s, color) = match code {
        AnsiCode::SetForegroundColor | AnsiCode::SetBackgroundColor => {
            let (s, _) = opt(tag(";"))(s)?;
            let (s, color) = color(s)?;
            (s, Some(color))
        }
        _ => (s, None),
    };
    let (s, _) = opt(tag(";"))(s)?;
    Ok((s, AnsiItem { code, color }))
}

fn color(s: &[u8]) -> IResult<&[u8], Color> {
    let (s, c_type) = color_type(s)?;
    let (s, _) = opt(tag(";"))(s)?;
    match c_type {
        ColorType::TrueColor => {
            let (s, (r, _, g, _, b)) = tuple((u8, tag(";"), u8, tag(";"), u8))(s)?;
            Ok((s, Color::Rgb(r, g, b)))
        }
        ColorType::EightBit => {
            let (s, index) = u8(s)?;
            Ok((s, Color::Indexed(index)))
        }
    }
}

fn color_type(s: &[u8]) -> IResult<&[u8], ColorType> {
    let (s, t) = i64(s)?;
    // NOTE: This isn't opt because a color type must always be followed by a color
    // let (s, _) = opt(tag(";"))(s)?;
    let (s, _) = tag(";")(s)?;
    match t {
        2 => Ok((s, ColorType::TrueColor)),
        5 => Ok((s, ColorType::EightBit)),
        _ => Err(nom::Err::Error(nom::error::Error::new(
            s,
            nom::error::ErrorKind::Alt,
        ))),
    }
}

fn take_until_next_esc_or_line_ending(
    line_ending: Option<&'_ str>,
) -> impl Fn(&[u8]) -> IResult<&[u8], &[u8]> + '_ {
    move |input: &[u8]| {
        let esc = b'\x1b';
        let le_bytes = line_ending.map(|le| le.as_bytes());
        let pos = input
            .iter()
            .enumerate()
            .find_map(|(i, &b)| {
                // check for the escape byte
                if b == esc {
                    Some(i)
                // check for the escape byte prepended by the carriage return
                } else if clear_line_code(&input[i..]).is_ok() {
                    Some(i)
                // last check, if we supplied a line ending
                } else if let Some(le) = le_bytes {
                    // check if it matches
                    if !le.is_empty()
                        && i + le.len() <= input.len()
                        && &input[i..i + le.len()] == le
                    {
                        Some(i)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or(input.len());
        Ok((&input[pos..], &input[..pos]))
    }
}

fn take_until_line_ending(
    line_ending: Option<&'_ str>,
    consume: bool,
) -> impl Fn(&[u8]) -> IResult<&[u8], &[u8]> + '_ {
    move |input: &[u8]| {
        if let Some(le) = line_ending {
            let le_bytes = le.as_bytes();
            if le_bytes.is_empty() {
                // No line ending bytes, return whole input
                return Ok((&input[0..0], input));
            }
            if let Some(pos) = input.windows(le_bytes.len()).position(|w| w == le_bytes) {
                let end = pos;
                let after = if consume && end + le_bytes.len() <= input.len() {
                    &input[end + le_bytes.len()..]
                } else {
                    &input[end..]
                };
                Ok((after, &input[..end]))
            } else {
                Ok((&input[input.len()..], input))
            }
        } else {
            Ok((&input[input.len()..], input))
        }
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn test_take_until_esc_or_line_ending_esc() {
        let input = b"Hello\x1b[1mWorld";
        let f = take_until_next_esc_or_line_ending(None);
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"Hello");
        assert_eq!(rest, b"\x1b[1mWorld");
    }

    #[test]
    fn test_take_until_esc_or_line_ending_line_ending_found() {
        let input = b"foo\r\nbar";
        let f = take_until_next_esc_or_line_ending(Some("\r\n"));
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"foo");
        assert_eq!(rest, b"\r\nbar");
    }

    #[test]
    fn test_take_until_esc_or_line_ending_line_ending_not_found() {
        let input = b"foo";
        let f = take_until_next_esc_or_line_ending(Some("\n"));
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"foo");
        assert_eq!(rest, b"");
    }

    #[test]
    fn test_take_until_esc_or_line_ending_both_present_picks_earliest() {
        let input = b"xx\x1bfoo\nbar";
        let f = take_until_next_esc_or_line_ending(Some("\n"));
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"xx");
        assert_eq!(rest, b"\x1bfoo\nbar");

        let input = b"abc\ndef\x1bgh";
        let f = take_until_next_esc_or_line_ending(Some("\n"));
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"abc");
        assert_eq!(rest, b"\ndef\x1bgh");
    }

    #[test]
    fn test_take_until_esc_or_line_ending_empty_input() {
        let input = b"";
        let f = take_until_next_esc_or_line_ending(Some("\n"));
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"");
        assert_eq!(rest, b"");
    }

    #[test]
    fn test_take_until_line_ending_none() {
        let input = b"abc";
        let f = take_until_line_ending(None, false);
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"abc");
        assert_eq!(rest, b"");
    }

    #[test]
    fn test_take_until_line_ending_basic() {
        let input = b"hello\nworld";
        let f = take_until_line_ending(Some("\n"), false);
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"hello");
        assert_eq!(rest, b"\nworld");
    }

    #[test]
    fn test_take_until_line_ending_consume_true() {
        let input = b"foo\r\nbar";
        let f = take_until_line_ending(Some("\r\n"), true);
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"foo");
        assert_eq!(rest, b"bar");
    }

    #[test]
    fn test_take_until_line_ending_not_found() {
        let input = b"foo";
        let f = take_until_line_ending(Some("\n"), false);
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"foo");
        assert_eq!(rest, b"");
    }

    #[test]
    fn test_take_until_line_ending_line_ending_empty() {
        let input = b"foo";
        let f = take_until_line_ending(Some(""), false);
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"foo");
        assert_eq!(rest, b"");
    }
}

#[cfg(test)]
mod ansi_tests {
    use super::*;

    #[test]
    fn color_test() {
        let c = color(b"2;255;255;255").unwrap();
        assert_eq!(c.1, Color::Rgb(255, 255, 255));
        let c = color(b"5;255").unwrap();
        assert_eq!(c.1, Color::Indexed(255));
        let err = color(b"10;255");
        assert_ne!(err, Ok(c));
    }

    #[test]
    fn ansi_items_test() {
        let sc = Default::default();
        let t = style(sc)(b"\x1b[38;2;3;3;3m").unwrap().1.unwrap();
        use ValueOrClear::Value;
        assert_eq!(
            t,
            Value(Style::from(AnsiStates {
                style: sc,
                items: vec![AnsiItem {
                    code: AnsiCode::SetForegroundColor,
                    color: Some(Color::Rgb(3, 3, 3))
                }]
                .into()
            }))
        );
        assert_eq!(
            style(sc)(b"\x1b[38;5;3m").unwrap().1.unwrap(),
            Value(Style::from(AnsiStates {
                style: sc,
                items: vec![AnsiItem {
                    code: AnsiCode::SetForegroundColor,
                    color: Some(Color::Indexed(3))
                }]
                .into()
            }))
        );
        assert_eq!(
            style(sc)(b"\x1b[38;5;3;48;5;3m").unwrap().1.unwrap(),
            Value(Style::from(AnsiStates {
                style: sc,
                items: vec![
                    AnsiItem {
                        code: AnsiCode::SetForegroundColor,
                        color: Some(Color::Indexed(3))
                    },
                    AnsiItem {
                        code: AnsiCode::SetBackgroundColor,
                        color: Some(Color::Indexed(3))
                    }
                ]
                .into()
            }))
        );
        assert_eq!(
            style(sc)(b"\x1b[38;5;3;48;5;3;1m").unwrap().1.unwrap(),
            Value(Style::from(AnsiStates {
                style: sc,
                items: vec![
                    AnsiItem {
                        code: AnsiCode::SetForegroundColor,
                        color: Some(Color::Indexed(3))
                    },
                    AnsiItem {
                        code: AnsiCode::SetBackgroundColor,
                        color: Some(Color::Indexed(3))
                    },
                    AnsiItem {
                        code: AnsiCode::Bold,
                        color: None
                    }
                ]
                .into()
            }))
        );
    }
}
