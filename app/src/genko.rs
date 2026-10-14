//! The sentence on manuscript paper (原稿用紙), written down the page: one
//! char per square, the reading of kanji in the narrow strip beside their
//! squares. Lines are laid out here; the CSS turns them into columns
//! (`writing-mode`).
//!
//! The layout depends on the sentence only, so the sheet is built once per
//! sentence; each square's and reading kana's class follows the progress
//! through its own memo, so a key changes only the squares whose state
//! does.

use leptos::prelude::*;
use wantype_core::{Progress, Segment};

/// One square.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Square {
    /// `None` for the blank squares that fill the last line.
    pub ch: Option<char>,
    /// The segment the char belongs to; `None` for a blank square.
    pub segment: Option<usize>,
    /// The first square of its segment (where the key hint goes).
    pub first: bool,
}

/// The reading of segment `segment`, a kanji group, over squares
/// `start..start + len`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub start: usize,
    pub len: usize,
    pub segment: usize,
    pub kana: Vec<char>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    pub squares: Vec<Square>,
    pub readings: Vec<Reading>,
}

/// Chars that may not begin a line (行頭禁則): written in the last square
/// of the line before instead, as on manuscript paper.
const NO_LINE_START: &str = "、。，．」』）〕】》〉！？!?)";

/// `segments` on lines of `width` squares. A kanji group stays on one line
/// with its reading. Closing punctuation never starts a line (行頭禁則):
/// the char before it goes along to the next line (追い出し), leaving a
/// blank square, so every char stays in its own square inside the sheet.
/// Lines are filled with blank squares.
pub fn lines(segments: &[Segment], width: usize) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut line = Line::default();
    for (index, segment) in segments.iter().enumerate() {
        let square = |(i, ch)| Square {
            ch: Some(ch),
            segment: Some(index),
            first: i == 0,
        };
        let (squares, kana): (Vec<Square>, Option<Vec<char>>) = match segment {
            Segment::Plain(t) => (vec![square((0, t.ch))], None),
            Segment::Ruby { text, reading, .. } => (
                text.chars().enumerate().map(square).collect(),
                Some(reading.iter().map(|t| t.ch).collect()),
            ),
        };
        if line.squares.len() + squares.len() > width && !line.squares.is_empty() {
            let closing = matches!(squares.as_slice(), [only] if closes(only));
            let carried = match closing {
                true => carry(&mut line),
                false => Line::default(),
            };
            lines.push(padded(std::mem::replace(&mut line, carried), width));
        }
        if let Some(kana) = kana {
            line.readings.push(Reading {
                start: line.squares.len(),
                len: squares.len(),
                segment: index,
                kana,
            });
        }
        line.squares.extend(squares);
    }
    lines.push(padded(line, width));
    lines
}

/// The square holds closing punctuation, which may not start a line.
fn closes(square: &Square) -> bool {
    square.ch.is_some_and(|c| NO_LINE_START.contains(c))
}

/// The end of `line` that goes along to the next line when closing
/// punctuation comes next: its last segment, and the ones before while the
/// carried part would itself start with closing punctuation (`」。`).
/// Nothing if that would take the whole line: the punctuation then starts
/// the next one, the lesser evil.
fn carry(line: &mut Line) -> Line {
    let mut at = line.squares.len();
    while let Some(before) = line.squares.get(..at) {
        let Some(segment) = before.last().and_then(|s| s.segment) else {
            break;
        };
        // Where that segment starts.
        at = before
            .iter()
            .rposition(|s| s.segment != Some(segment))
            .map_or(0, |i| i + 1);
        if !line.squares.get(at).is_some_and(closes) {
            break;
        }
    }
    if at == 0 {
        return Line::default();
    }
    let squares = line.squares.split_off(at);
    let (carried, kept): (Vec<Reading>, Vec<Reading>) = std::mem::take(&mut line.readings)
        .into_iter()
        .partition(|r| r.start >= at);
    line.readings = kept;
    let readings = carried
        .into_iter()
        .map(|r| Reading {
            start: r.start - at,
            ..r
        })
        .collect();
    Line { squares, readings }
}

/// `line` filled with blank squares to `width`.
fn padded(mut line: Line, width: usize) -> Line {
    while line.squares.len() < width {
        line.squares.push(Square {
            ch: None,
            segment: None,
            first: false,
        });
    }
    line
}

/// Squares per column: ten down, so that a column fits the card.
pub const WIDTH: usize = 10;

/// The squares a segment takes: one per char.
fn width_of(segment: &Segment) -> usize {
    match segment {
        Segment::Plain(_) => 1,
        Segment::Ruby { text, .. } => text.chars().count(),
    }
}

/// The squares `segments` take.
pub fn squares(segments: &[Segment]) -> usize {
    segments.iter().map(width_of).sum()
}

/// The first square of the segment being typed; past the last once all
/// are typed.
pub fn active_square(segments: &[Segment]) -> usize {
    segments
        .iter()
        .take_while(|s| state_of(s).0 != Progress::Active)
        .map(width_of)
        .sum()
}

/// A segment's progress and whether a key was missed in it.
fn state_of(segment: &Segment) -> (Progress, bool) {
    match segment {
        Segment::Plain(t) => (t.progress, t.error),
        Segment::Ruby {
            progress, error, ..
        } => (*progress, *error),
    }
}

fn class(progress: Progress) -> &'static str {
    match progress {
        Progress::Done => "done",
        Progress::Active => "active",
        Progress::Todo => "todo",
    }
}

/// Segment `index`'s progress and whether a key was missed in it.
fn state(segments: &[Segment], index: usize) -> Option<(Progress, bool)> {
    segments.get(index).map(state_of)
}

/// The progress of kana `kana` of segment `index`'s reading.
fn kana_progress(segments: &[Segment], index: usize, kana: usize) -> Option<Progress> {
    match segments.get(index)? {
        Segment::Ruby { reading, .. } => reading.get(kana).map(|t| t.progress),
        Segment::Plain(_) => None,
    }
}

/// The sheet for `lines`, its classes following `segments`. Squares are
/// placed in the grid's second row in order; readings span their kanji's
/// squares in the first. `hint` shows at the first square of the active
/// segment.
pub fn view(
    lines: Vec<Line>,
    width: usize,
    segments: Memo<Vec<Segment>>,
    hint: Signal<Option<char>>,
) -> impl IntoView {
    let lines = lines
        .into_iter()
        .map(|line| {
            let readings = line
                .readings
                .into_iter()
                .map(|reading| {
                    let index = reading.segment;
                    // The CSS shrinks a reading longer than its squares
                    // (じょう beside 状) to fit them.
                    let count = reading.kana.len().to_string();
                    let kana = reading
                        .kana
                        .into_iter()
                        .enumerate()
                        .map(|(j, ch)| {
                            let class = Memo::new(move |_| {
                                segments.with(|s| kana_progress(s, index, j).map_or("todo", class))
                            });
                            view! { <span class=class>{ch.to_string()}</span> }
                        })
                        .collect_view();
                    let column = format!("{} / span {}", reading.start + 1, reading.len);
                    view! {
                        <span
                            class="genko-reading"
                            style:grid-column=column
                            style:--span=reading.len.to_string()
                            style:--kana=count
                        >
                            {kana}
                        </span>
                    }
                })
                .collect_view();
            let squares = line
                .squares
                .into_iter()
                .enumerate()
                .map(|(i, square)| {
                    let hang = i >= width;
                    let Some(index) = square.segment else {
                        return view! { <span class="genko-square todo"></span> }.into_any();
                    };
                    let current = Memo::new(move |_| segments.with(|s| state(s, index)));
                    // One class string: `class:` toggles beside it are lost
                    // when the string changes.
                    let classes = move || {
                        let (progress, error) = current.get().unwrap_or((Progress::Todo, false));
                        let mut classes = format!("genko-square {}", class(progress));
                        for (on, name) in [(error, " error"), (hang, " hang")] {
                            if on {
                                classes.push_str(name);
                            }
                        }
                        classes
                    };
                    let letter = move || {
                        let active = matches!(current.get(), Some((Progress::Active, _)));
                        (square.first && active)
                            .then(|| hint.get())
                            .flatten()
                            .map(|c| view! { <span class="genko-hint">{c.to_string()}</span> })
                    };
                    view! {
                        <span class=classes>
                            {square.ch.map(String::from)}
                            {letter}
                        </span>
                    }
                    .into_any()
                })
                .collect_view();
            view! { <div class="genko-line">{readings}{squares}</div> }
        })
        .collect_view();
    view! { <div class="genko-sheet" style:--cells=width.to_string()>{lines}</div> }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use wantype_core::{segments, ParseError, Sentence};

    use super::*;

    fn layout(
        transcription: &str,
        typed: usize,
        width: usize,
    ) -> Result<(Vec<Line>, Vec<Segment>), ParseError> {
        let sentence = Sentence::parse(None, transcription, BTreeMap::new(), None)?;
        let segments = segments(&sentence, typed, |_| false, |_| false);
        Ok((lines(&segments, width), segments))
    }

    fn text(line: &Line) -> String {
        line.squares.iter().map(|s| s.ch.unwrap_or('□')).collect()
    }

    #[test]
    fn the_active_square_is_the_first_of_its_segment() -> Result<(), ParseError> {
        // 日本 is one segment of two squares, read にほん.
        let at =
            |typed| layout("これは[日本|にほん]です。", typed, 99).map(|(_, s)| active_square(&s));
        assert_eq!(at(0)?, 0);
        assert_eq!(at(3)?, 3);
        // Halfway through にほん the square to type is still 日.
        assert_eq!(at(4)?, 3);
        assert_eq!(at(6)?, 5);
        let (_, all) = layout("これは[日本|にほん]です。", 9, 99)?;
        assert_eq!((active_square(&all), squares(&all)), (8, 8));
        Ok(())
    }

    #[test]
    fn fills_lines_and_pads_the_last() -> Result<(), ParseError> {
        let (lines, _) = layout("これは[日|に][本|ほん][語|ご]です。", 0, 5)?;
        let texts: Vec<String> = lines.iter().map(text).collect();
        assert_eq!(texts, ["これは日本", "語です。□"]);
        assert_eq!(
            lines[0]
                .readings
                .iter()
                .map(|r| (r.start, r.len))
                .collect::<Vec<_>>(),
            [(3, 1), (4, 1)]
        );
        assert_eq!(lines[1].readings[0].start, 0);
        Ok(())
    }

    #[test]
    fn a_kanji_group_moves_to_the_next_line_whole() -> Result<(), ParseError> {
        let (lines, _) = layout("あいう[田舎|いなか]", 0, 4)?;
        assert_eq!(text(&lines[0]), "あいう□", "every line has all its squares");
        assert_eq!(text(&lines[1]), "田舎□□");
        assert_eq!(lines[1].readings[0].len, 2);
        let firsts: Vec<bool> = lines[1].squares.iter().map(|s| s.first).collect();
        assert_eq!(firsts, [true, false, false, false], "the hint goes on 田");
        Ok(())
    }

    #[test]
    fn closing_punctuation_never_starts_a_line() -> Result<(), ParseError> {
        let texts = |transcription, width| -> Result<Vec<String>, ParseError> {
            Ok(layout(transcription, 0, width)?
                .0
                .iter()
                .map(text)
                .collect())
        };
        // The char before it goes along (追い出し), inside the sheet.
        assert_eq!(texts("あいう。", 3)?, ["あい□", "う。□"]);
        assert_eq!(
            texts("トムって[呼|よ]んでもいい？", 10)?,
            ["トムって呼んでもい□", "い？□□□□□□□□"]
        );
        // A kanji group goes along whole, its reading with it.
        let (lines, _) = layout("あい[日本|にほん]。", 0, 4)?;
        assert_eq!(text(&lines[1]), "日本。□");
        assert_eq!(lines[1].readings[0].start, 0);
        // Closing marks in a row go along together.
        assert_eq!(texts("あいう」。", 4)?, ["あい□□", "う」。□"]);
        // Only when the whole line would go does the mark start the next.
        assert_eq!(texts("あ」。", 2)?, ["あ」", "。□"]);
        Ok(())
    }

    /// The layout does not depend on progress: the sheet is built once per
    /// sentence, and its squares look their state up as keys come.
    #[test]
    fn squares_follow_the_matcher_through_their_segment() -> Result<(), ParseError> {
        let (before, _) = layout("か[日|ひ]", 0, 4)?;
        let (lines, segments) = layout("か[日|ひ]", 1, 4)?;
        assert_eq!(lines, before);
        let progress: Vec<Option<Progress>> = lines[0]
            .squares
            .iter()
            .map(|s| s.segment.and_then(|i| state(&segments, i)).map(|(p, _)| p))
            .collect();
        assert_eq!(
            progress[..3],
            [Some(Progress::Done), Some(Progress::Active), None],
            "the kanji is active while its reading is typed"
        );
        assert_eq!(kana_progress(&segments, 1, 0), Some(Progress::Active));
        Ok(())
    }
}
