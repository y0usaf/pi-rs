//! Offline checker for the compact canonical-experience fixture format.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};

const FORMAT: &str = "pi-rs-experience-grid";
const VERSION: u16 = 1;
const EMPTY: char = '░';
const SPACE: char = '␠';
const REQUIRED_COVERAGE: [&str; 9] = [
    "startup",
    "prompt-editing",
    "streaming",
    "thinking",
    "tool-call-result",
    "queueing",
    "cancellation",
    "selector-dialog",
    "session-resume",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    format: String,
    version: u16,
    oracle: Oracle,
    styles: BTreeMap<String, Style>,
    journeys: Vec<Journey>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Oracle {
    product: String,
    revision: String,
    terminal: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Style {
    #[serde(skip_serializing_if = "Option::is_none")]
    foreground: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    dim: bool,
    #[serde(skip_serializing_if = "is_false")]
    italic: bool,
    #[serde(skip_serializing_if = "is_false")]
    underline: bool,
    #[serde(skip_serializing_if = "is_false")]
    inverse: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journey {
    name: String,
    covers: Vec<String>,
    source: String,
    steps: Vec<Step>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Step {
    name: String,
    from: String,
    input: Vec<Input>,
    frame: Frame,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", tag = "type", content = "value")]
enum Input {
    Text(String),
    Key(String),
    BytesHex(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    size: [u16; 2],
    cursor: Cursor,
    /// One character per terminal cell. `░` = untouched empty cell and
    /// `␠` = a written space. Missing row tails/rows are empty cells.
    glyphs: Vec<String>,
    /// `[row, start_column, end_column_exclusive, style_name]`.
    styles: Vec<(u16, u16, u16, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    wide: Vec<(u16, u16)>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    row: u16,
    column: u16,
    visible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Cell {
    glyph: String,
    style: Style,
    wide: bool,
    wide_continuation: bool,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("invalid fixture: {0}")]
    Invalid(String),
    #[error("fixture is not byte-idempotent: {0}")]
    NotCanonical(PathBuf),
    #[error("{0}")]
    Mismatch(String),
    #[error(
        "usage: ui-diff --check FIXTURE | --compare EXPECTED ACTUAL | --self-test FIXTURE | --canonicalize FIXTURE"
    )]
    Usage,
}

fn read_fixture(path: &Path) -> Result<(Fixture, Vec<u8>), Error> {
    let bytes = fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let fixture = serde_json::from_slice(&bytes).map_err(|source| Error::Json {
        path: path.to_path_buf(),
        source,
    })?;
    validate(&fixture)?;
    Ok((fixture, bytes))
}

fn canonical_bytes(fixture: &Fixture) -> Result<Vec<u8>, Error> {
    let mut bytes = serde_json::to_vec_pretty(fixture)
        .map_err(|error| Error::Invalid(format!("serialization failed: {error}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn input_bytes(input: &Input) -> Result<Vec<u8>, Error> {
    match input {
        Input::Text(text) => Ok(text.as_bytes().to_vec()),
        Input::Key(key) => match key.as_str() {
            "enter" => Ok(vec![b'\r']),
            "escape" => Ok(vec![0x1b]),
            "ctrl-c" => Ok(vec![0x03]),
            "ctrl-o" => Ok(vec![0x0f]),
            "ctrl-t" => Ok(vec![0x14]),
            "alt-enter" => Ok(b"\x1b[13;3u".to_vec()),
            "up" => Ok(b"\x1b[A".to_vec()),
            "down" => Ok(b"\x1b[B".to_vec()),
            "ctrl-left" => Ok(b"\x1b[1;5D".to_vec()),
            other => Err(Error::Invalid(format!("unknown input key {other:?}"))),
        },
        Input::BytesHex(hex) => decode_hex(hex),
    }
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, Error> {
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Invalid(format!(
            "bytes-hex must contain an even number of hexadecimal digits: {hex:?}"
        )));
    }
    (0..hex.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&hex[index..index + 2], 16)
                .map_err(|error| Error::Invalid(format!("invalid bytes-hex: {error}")))
        })
        .collect()
}

fn validate(fixture: &Fixture) -> Result<(), Error> {
    if fixture.format != FORMAT || fixture.version != VERSION {
        return Err(Error::Invalid(format!(
            "expected format={FORMAT:?} version={VERSION}, got format={:?} version={}",
            fixture.format, fixture.version
        )));
    }
    if fixture.oracle.product != "Pi v0.79.0"
        || fixture.oracle.revision != "c5582102f51b143fadc05180e0f8aed050e923b3"
    {
        return Err(Error::Invalid(
            "oracle must identify pinned Pi v0.79.0 c5582102".to_owned(),
        ));
    }
    let mut coverage = BTreeSet::new();
    let mut journey_names = BTreeSet::new();
    for journey in &fixture.journeys {
        if !journey_names.insert(&journey.name) {
            return Err(Error::Invalid(format!(
                "duplicate journey {:?}",
                journey.name
            )));
        }
        coverage.extend(journey.covers.iter().map(String::as_str));
        let mut step_names = BTreeSet::new();
        for step in &journey.steps {
            if !step_names.insert(&step.name) {
                return Err(Error::Invalid(format!(
                    "duplicate step {:?} in journey {:?}",
                    step.name, journey.name
                )));
            }
            for input in &step.input {
                let _ = input_bytes(input)?;
            }
            let _ = decode_frame(&step.frame, &fixture.styles).map_err(|error| {
                Error::Invalid(format!(
                    "journey {:?} step {:?}: {error}",
                    journey.name, step.name
                ))
            })?;
        }
    }
    let missing: Vec<_> = REQUIRED_COVERAGE
        .iter()
        .filter(|name| !coverage.contains(**name))
        .collect();
    if !missing.is_empty() {
        return Err(Error::Invalid(format!(
            "missing canonical coverage: {}",
            missing
                .iter()
                .map(|name| format!("{name:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(())
}

fn decode_frame(frame: &Frame, palette: &BTreeMap<String, Style>) -> Result<Vec<Cell>, String> {
    let [columns, rows] = frame.size;
    if columns == 0 || rows == 0 {
        return Err("frame dimensions must be non-zero".to_owned());
    }
    if frame.cursor.row >= rows || frame.cursor.column > columns {
        return Err(format!(
            "cursor ({},{}) is outside {}x{} frame",
            frame.cursor.row, frame.cursor.column, columns, rows
        ));
    }
    if frame.glyphs.len() > usize::from(rows) {
        return Err("glyph row count exceeds frame height".to_owned());
    }
    let mut cells = vec![
        Cell {
            glyph: String::new(),
            style: Style::default(),
            wide: false,
            wide_continuation: false,
        };
        usize::from(columns) * usize::from(rows)
    ];
    for (row, glyphs) in frame.glyphs.iter().enumerate() {
        let encoded: Vec<char> = glyphs.chars().collect();
        if encoded.len() > usize::from(columns) {
            return Err(format!("glyph row {row} exceeds frame width"));
        }
        for (column, glyph) in encoded.into_iter().enumerate() {
            cells[row * usize::from(columns) + column].glyph = match glyph {
                EMPTY => String::new(),
                SPACE => " ".to_owned(),
                other => other.to_string(),
            };
        }
    }
    for (row, start, end, style_name) in &frame.styles {
        if *row >= rows || start >= end || *end > columns {
            return Err(format!(
                "invalid style span [{row},{start},{end},{style_name:?}]"
            ));
        }
        let style = palette
            .get(style_name)
            .ok_or_else(|| format!("unknown style {style_name:?}"))?;
        for column in *start..*end {
            cells[usize::from(*row) * usize::from(columns) + usize::from(column)].style =
                style.clone();
        }
    }
    let wide: BTreeSet<_> = frame.wide.iter().copied().collect();
    for (row, column) in wide {
        if row >= rows || column + 1 >= columns {
            return Err(format!("invalid wide cell ({row},{column})"));
        }
        let index = usize::from(row) * usize::from(columns) + usize::from(column);
        cells[index].wide = true;
        cells[index + 1].wide_continuation = true;
    }
    Ok(cells)
}

fn compare(expected: &Fixture, actual: &Fixture) -> Result<(), Error> {
    if expected.journeys.len() != actual.journeys.len() {
        return Err(Error::Mismatch(format!(
            "journey count differs: expected {}, got {}",
            expected.journeys.len(),
            actual.journeys.len()
        )));
    }
    for (journey_index, (expected_journey, actual_journey)) in
        expected.journeys.iter().zip(&actual.journeys).enumerate()
    {
        if expected_journey.name != actual_journey.name {
            return Err(Error::Mismatch(format!(
                "journey {journey_index} name differs: expected {:?}, got {:?}",
                expected_journey.name, actual_journey.name
            )));
        }
        if expected_journey.steps.len() != actual_journey.steps.len() {
            return Err(Error::Mismatch(format!(
                "step count differs journey={:?}: expected {}, got {}",
                expected_journey.name,
                expected_journey.steps.len(),
                actual_journey.steps.len()
            )));
        }
        for (step_index, (expected_step, actual_step)) in expected_journey
            .steps
            .iter()
            .zip(&actual_journey.steps)
            .enumerate()
        {
            if expected_step.name != actual_step.name {
                return Err(Error::Mismatch(format!(
                    "step {step_index} name differs journey={:?}: expected {:?}, got {:?}",
                    expected_journey.name, expected_step.name, actual_step.name
                )));
            }
            compare_input(&expected_journey.name, expected_step, actual_step)?;
            compare_frame(
                &expected_journey.name,
                expected_step,
                actual_step,
                &expected.styles,
                &actual.styles,
            )?;
        }
    }
    Ok(())
}

fn compare_input(journey: &str, expected: &Step, actual: &Step) -> Result<(), Error> {
    let expected_bytes = expected
        .input
        .iter()
        .map(input_bytes)
        .collect::<Result<Vec<_>, _>>()?
        .concat();
    let actual_bytes = actual
        .input
        .iter()
        .map(input_bytes)
        .collect::<Result<Vec<_>, _>>()?
        .concat();
    let common = expected_bytes.len().min(actual_bytes.len());
    for index in 0..common {
        if expected_bytes[index] != actual_bytes[index] {
            return Err(Error::Mismatch(format!(
                "input mismatch journey={journey:?} step={:?} byte={index}: expected 0x{:02x}, got 0x{:02x}",
                expected.name, expected_bytes[index], actual_bytes[index]
            )));
        }
    }
    if expected_bytes.len() != actual_bytes.len() {
        return Err(Error::Mismatch(format!(
            "input mismatch journey={journey:?} step={:?} byte={common}: expected length {}, got {}",
            expected.name,
            expected_bytes.len(),
            actual_bytes.len()
        )));
    }
    Ok(())
}

fn compare_frame(
    journey: &str,
    expected: &Step,
    actual: &Step,
    expected_palette: &BTreeMap<String, Style>,
    actual_palette: &BTreeMap<String, Style>,
) -> Result<(), Error> {
    if expected.frame.size != actual.frame.size {
        return Err(Error::Mismatch(format!(
            "frame size mismatch journey={journey:?} step={:?}: expected {:?}, got {:?}",
            expected.name, expected.frame.size, actual.frame.size
        )));
    }
    if expected.frame.cursor != actual.frame.cursor {
        return Err(Error::Mismatch(format!(
            "cursor mismatch journey={journey:?} step={:?}: expected {:?}, got {:?}",
            expected.name, expected.frame.cursor, actual.frame.cursor
        )));
    }
    let expected_cells = decode_frame(&expected.frame, expected_palette).map_err(Error::Invalid)?;
    let actual_cells = decode_frame(&actual.frame, actual_palette).map_err(Error::Invalid)?;
    for (index, (expected_cell, actual_cell)) in
        expected_cells.iter().zip(&actual_cells).enumerate()
    {
        if expected_cell != actual_cell {
            let columns = usize::from(expected.frame.size[0]);
            return Err(Error::Mismatch(format!(
                "cell mismatch journey={journey:?} step={:?} row={} column={}: expected {:?}, got {:?}",
                expected.name,
                index / columns,
                index % columns,
                expected_cell,
                actual_cell
            )));
        }
    }
    Ok(())
}

fn first_input_mut(fixture: &mut Fixture) -> Option<&mut Input> {
    fixture
        .journeys
        .iter_mut()
        .flat_map(|journey| &mut journey.steps)
        .flat_map(|step| &mut step.input)
        .next()
}

fn self_test(fixture: &Fixture) -> Result<(String, String), Error> {
    let mut bad_cell = fixture.clone();
    let first_frame = bad_cell
        .journeys
        .first_mut()
        .and_then(|journey| journey.steps.first_mut())
        .ok_or_else(|| Error::Invalid("self-test requires a frame".to_owned()))?;
    if first_frame.frame.glyphs.is_empty() || first_frame.frame.glyphs[0].is_empty() {
        first_frame.frame.glyphs = vec!["X".to_owned()];
    } else {
        let first_len = first_frame.frame.glyphs[0]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
        first_frame.frame.glyphs[0].replace_range(..first_len, "X");
    }
    let cell = match compare(fixture, &bad_cell) {
        Err(Error::Mismatch(message)) if message.starts_with("cell mismatch") => message,
        Err(error) => return Err(Error::Invalid(format!("cell negative control: {error}"))),
        Ok(()) => return Err(Error::Invalid("cell negative control matched".to_owned())),
    };

    let mut bad_input = fixture.clone();
    let input = first_input_mut(&mut bad_input)
        .ok_or_else(|| Error::Invalid("self-test requires an input".to_owned()))?;
    *input = Input::BytesHex("ff".to_owned());
    let input = match compare(fixture, &bad_input) {
        Err(Error::Mismatch(message)) if message.starts_with("input mismatch") => message,
        Err(error) => return Err(Error::Invalid(format!("input negative control: {error}"))),
        Ok(()) => return Err(Error::Invalid("input negative control matched".to_owned())),
    };
    Ok((cell, input))
}

fn run() -> Result<(), Error> {
    let mut args = std::env::args_os().skip(1);
    match args
        .next()
        .and_then(|arg| arg.into_string().ok())
        .as_deref()
    {
        Some("--check") => {
            let path = PathBuf::from(args.next().ok_or(Error::Usage)?);
            if args.next().is_some() {
                return Err(Error::Usage);
            }
            let (fixture, original) = read_fixture(&path)?;
            if canonical_bytes(&fixture)? != original {
                return Err(Error::NotCanonical(path));
            }
            println!(
                "experience fixture valid: {} journeys, {} steps",
                fixture.journeys.len(),
                fixture
                    .journeys
                    .iter()
                    .map(|journey| journey.steps.len())
                    .sum::<usize>()
            );
            Ok(())
        }
        Some("--compare") => {
            let expected_path = PathBuf::from(args.next().ok_or(Error::Usage)?);
            let actual_path = PathBuf::from(args.next().ok_or(Error::Usage)?);
            if args.next().is_some() {
                return Err(Error::Usage);
            }
            let (expected, _) = read_fixture(&expected_path)?;
            let (actual, _) = read_fixture(&actual_path)?;
            compare(&expected, &actual)?;
            println!("experience fixtures match");
            Ok(())
        }
        Some("--self-test") => {
            let path = PathBuf::from(args.next().ok_or(Error::Usage)?);
            if args.next().is_some() {
                return Err(Error::Usage);
            }
            let (fixture, _) = read_fixture(&path)?;
            let (cell, input) = self_test(&fixture)?;
            println!("cell-negative: {cell}");
            println!("input-negative: {input}");
            Ok(())
        }
        Some("--canonicalize") => {
            let path = PathBuf::from(args.next().ok_or(Error::Usage)?);
            if args.next().is_some() {
                return Err(Error::Usage);
            }
            let (fixture, _) = read_fixture(&path)?;
            fs::write(&path, canonical_bytes(&fixture)?).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            println!("canonicalized {}", path.display());
            Ok(())
        }
        _ => Err(Error::Usage),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ui-diff: {error}");
            ExitCode::FAILURE
        }
    }
}
