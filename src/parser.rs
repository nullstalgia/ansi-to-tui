use crate::code::AnsiCode;
use nom::{
    branch::alt,
    bytes::complete::*,
    character::{complete::*, is_alphabetic},
    combinator::{map_res, opt, recognize, value},
    error::{self, Error, ErrorKind, FromExternalError},
    multi::*,
    sequence::{delimited, preceded, terminated, tuple},
    IResult, Parser,
};
use std::str::FromStr;
use tui::{
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
};

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
    lossy: bool,
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
pub(crate) fn text_fast<'a>(mut s: &'a [u8], line_ending: &str) -> IResult<&'a [u8], Text<'a>> {
    let mut lines = Vec::new();
    let mut last = Style::new();
    while let Ok((_s, (line, style))) = line_fast(last, Some(line_ending))(s) {
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
    lossy: bool,
) -> impl Fn(&[u8]) -> IResult<&[u8], (Line<'static>, Style)> + '_ {
    move |s: &[u8]| -> IResult<&[u8], (Line<'static>, Style)> {
        let (s, mut text) = take_until_line_ending(line_ending, true)(s)?;

        let mut spans = Vec::new();
        let mut last = style;
        while let Ok((s, span)) = span(last, line_ending, lossy)(text) {
            match span {
                ValueOrClear::ClearLine => spans.clear(),
                ValueOrClear::Value(span) => {
                    // Since reset now tracks seperately we can skip the reset check
                    last = last.patch(span.style);

                    if !span.content.is_empty() {
                        spans.push(span);
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
fn line_fast(
    style: Style,
    line_ending: Option<&'_ str>,
) -> impl Fn(&[u8]) -> IResult<&[u8], (Line<'_>, Style)> + '_ {
    // let style_: Style = Default::default();
    move |s: &[u8]| -> IResult<&[u8], (Line<'_>, Style)> {
        let (s, mut text) = take_until_line_ending(line_ending, true)(s)?;
        let mut spans = Vec::new();
        let mut last = style;
        while let Ok((s, span)) = span_fast(last, line_ending)(text) {
            match span {
                ValueOrClear::ClearLine => spans.clear(),
                ValueOrClear::Value(span) => {
                    last = last.patch(span.style);
                    // If the spans is empty then it might be possible that the style changes
                    // but there is no text change
                    if !span.content.is_empty() {
                        spans.push(span);
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

#[allow(clippy::type_complexity)]
fn span(
    last: Style,
    line_ending: Option<&'_ str>,
    lossy: bool,
) -> impl Fn(&[u8]) -> IResult<&[u8], ValueOrClear<Span<'static>>, nom::error::Error<&[u8]>> + '_ {
    move |s: &[u8]| -> IResult<&[u8], ValueOrClear<Span<'static>>> {
        let mut last = last;
        let (s, style) = opt(style(last))(s)?;

        if let Some(Some(ValueOrClear::ClearLine)) = style {
            return Ok((s, ValueOrClear::ClearLine));
        }

        if lossy {
            let (s, text) = take_until_esc_or_line_ending(line_ending)(s)?;
            let text = String::from_utf8_lossy(text);
            if let Some(ValueOrClear::Value(style)) = style.flatten() {
                last = last.patch(style);
            }

            Ok((s, ValueOrClear::Value(Span::styled(text.to_string(), last))))
        } else {
            #[cfg(feature = "simd")]
            let (s, text) = map_res(take_until_esc_or_line_ending(line_ending), |t| {
                simdutf8::basic::from_utf8(t)
            })(s)?;

            #[cfg(not(feature = "simd"))]
            let (s, text) = map_res(take_until_esc_or_line_ending(line_ending), |t| {
                std::str::from_utf8(t)
            })(s)?;
            if let Some(ValueOrClear::Value(style)) = style.flatten() {
                last = last.patch(style);
            }

            Ok((s, ValueOrClear::Value(Span::styled(text.to_string(), last))))
        }
    }
}

#[cfg(feature = "zero-copy")]
#[allow(clippy::type_complexity)]
fn span_fast(
    last: Style,
    line_ending: Option<&'_ str>,
) -> impl Fn(&[u8]) -> IResult<&[u8], ValueOrClear<Span<'_>>, nom::error::Error<&[u8]>> + '_ {
    move |s: &[u8]| -> IResult<&[u8], ValueOrClear<Span<'_>>> {
        let mut last = last;
        let (s, style) = opt(style(last))(s)?;

        if let Some(Some(ValueOrClear::ClearLine)) = style {
            return Ok((s, ValueOrClear::ClearLine));
        }

        #[cfg(feature = "simd")]
        let (s, text) = map_res(take_until_esc_or_line_ending(line_ending), |t| {
            simdutf8::basic::from_utf8(t)
        })(s)?;

        #[cfg(not(feature = "simd"))]
        let (s, text) = map_res(take_until_esc_or_line_ending(line_ending), |t| {
            std::str::from_utf8(t)
        })(s)?;

        if let Some(ValueOrClear::Value(style)) = style.flatten() {
            last = last.patch(style);
        }

        Ok((s, ValueOrClear::Value(Span::styled(text, last))))
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
            (s, None) => {
                // if no style found, check for a clear line
                let (s, clear) = clear_line_code(s)?;
                if clear.is_some() {
                    return Ok((s, clear));
                }
                let (s, _) = any_escape_sequence(s)?;
                (s, None)
            }
        };
        Ok((s, r))
    }
}

/// Parse a Clear Line code
fn clear_line_code(
    s: &[u8],
) -> IResult<&[u8], Option<ValueOrClear<Style>>, nom::error::Error<&[u8]>> {
    // Match the ANSI 'EL' (Erase in Line) sequence: ESC [ K
    let (s, matched) = opt(tag("\x1b[K"))(s)?;
    if matched.is_some() {
        Ok((s, Some(ValueOrClear::ClearLine)))
    } else {
        Ok((s, None))
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

fn take_until_esc_or_line_ending(
    line_ending: Option<&'_ str>,
) -> impl Fn(&[u8]) -> IResult<&[u8], &[u8]> + '_ {
    move |input: &[u8]| {
        let esc = b'\x1b';
        let le_bytes = line_ending.map(|le| le.as_bytes());
        let pos = input
            .iter()
            .enumerate()
            .find_map(|(i, &b)| {
                if b == esc {
                    Some(i)
                } else if let Some(le) = le_bytes {
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
        let f = take_until_esc_or_line_ending(None);
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"Hello");
        assert_eq!(rest, b"\x1b[1mWorld");
    }

    #[test]
    fn test_take_until_esc_or_line_ending_line_ending_found() {
        let input = b"foo\r\nbar";
        let f = take_until_esc_or_line_ending(Some("\r\n"));
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"foo");
        assert_eq!(rest, b"\r\nbar");
    }

    #[test]
    fn test_take_until_esc_or_line_ending_line_ending_not_found() {
        let input = b"foo";
        let f = take_until_esc_or_line_ending(Some("\n"));
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"foo");
        assert_eq!(rest, b"");
    }

    #[test]
    fn test_take_until_esc_or_line_ending_both_present_picks_earliest() {
        let input = b"xx\x1bfoo\nbar";
        let f = take_until_esc_or_line_ending(Some("\n"));
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"xx");
        assert_eq!(rest, b"\x1bfoo\nbar");

        let input = b"abc\ndef\x1bgh";
        let f = take_until_esc_or_line_ending(Some("\n"));
        let (rest, out) = f(input).unwrap();
        assert_eq!(out, b"abc");
        assert_eq!(rest, b"\ndef\x1bgh");
    }

    #[test]
    fn test_take_until_esc_or_line_ending_empty_input() {
        let input = b"";
        let f = take_until_esc_or_line_ending(Some("\n"));
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
