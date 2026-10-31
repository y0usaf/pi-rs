use std::cell::Cell;
use std::rc::Rc;

use pi_tui::tui::{Component, OverlayAnchor, OverlayMargin, OverlayOptions, SizeValue};
use pi_tui::utils::{truncate_to_width, visible_width};
use serde::{Deserialize, Serialize};

use super::completions::ItemWire;
use crate::extensions::Live;

#[derive(Serialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum ToHost<'a> {
    Size {
        columns: usize,
        rows: usize,
    },
    Kitty {
        active: bool,
    },
    Expanded {
        expanded: bool,
    },
    Providers {
        count: usize,
    },
    Render {
        component: u64,
        width: usize,
    },
    Input {
        component: u64,
        data: &'a str,
    },
    Focus {
        component: u64,
        focused: bool,
    },
    Removed {
        component: u64,
    },
    TerminalInput {
        sequence: u64,
        data: &'a str,
    },
    EditorInput {
        component: u64,
        sequence: u64,
        data: &'a str,
    },
    EditorSetText {
        component: u64,
        text: &'a str,
    },
    EditorAddHistory {
        component: u64,
        text: &'a str,
    },
    EditorDropped {
        component: u64,
    },
    #[serde(rename_all = "camelCase")]
    EditorStyle {
        padding_x: usize,
        autocomplete_max_visible: usize,
        bash: bool,
        thinking: &'a str,
    },
    #[serde(rename_all = "camelCase")]
    AutocompleteSuggest {
        request: u64,
        lines: &'a [String],
        cursor_line: usize,
        cursor_col: usize,
        force: bool,
    },
    AutocompleteAbandon {
        request: u64,
    },
    #[serde(rename_all = "camelCase")]
    AutocompleteApply {
        request: u64,
        lines: &'a [String],
        cursor_line: usize,
        cursor_col: usize,
        item: ItemWire,
        prefix: &'a str,
    },
    #[serde(rename_all = "camelCase")]
    AutocompleteShouldTrigger {
        request: u64,
        lines: &'a [String],
        cursor_line: usize,
        cursor_col: usize,
    },
}

#[derive(Deserialize)]
pub struct Drawn {
    width: usize,
    lines: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostedWire {
    pub component: u64,
    input: bool,
    focusable: bool,
    key_release: bool,
    #[serde(default)]
    drawn: Option<Drawn>,
}

pub struct Hosted {
    live: &'static Live,
    component: u64,
    input: bool,
    focusable: bool,
    key_release: bool,
    asked: Option<usize>,
    lines: Vec<String>,
}

impl Hosted {
    pub fn new(live: &'static Live, wire: HostedWire) -> Self {
        let (asked, lines) = match wire.drawn {
            Some(drawn) => (Some(drawn.width), drawn.lines),
            None => (None, Vec::new()),
        };
        Hosted {
            live,
            component: wire.component,
            input: wire.input,
            focusable: wire.focusable,
            key_release: wire.key_release,
            asked,
            lines,
        }
    }

    pub fn draw(&mut self, width: usize, lines: Vec<String>) -> bool {
        if self.asked != Some(width) {
            return false;
        }
        self.lines = lines;
        true
    }

    pub fn request(&mut self, width: usize) {
        if self.asked == Some(width) {
            return;
        }
        self.asked = Some(width);
        self.live.ui(&ToHost::Render {
            component: self.component,
            width,
        });
    }
}

impl Component for Hosted {
    fn render(&mut self, width: usize) -> Vec<String> {
        self.request(width);
        self.lines
            .iter()
            .map(|line| {
                if visible_width(line) > width {
                    truncate_to_width(line, width, "", false)
                } else {
                    line.clone()
                }
            })
            .collect()
    }

    fn handle_input(&mut self, data: &str) {
        self.live.ui(&ToHost::Input {
            component: self.component,
            data,
        });
    }

    fn accepts_input(&self) -> bool {
        self.input
    }

    fn wants_key_release(&self) -> bool {
        self.key_release
    }

    fn invalidate(&mut self) {
        self.asked = None;
    }

    fn set_focused(&mut self, focused: bool) {
        self.live.ui(&ToHost::Focus {
            component: self.component,
            focused,
        });
    }

    fn is_focusable(&self) -> bool {
        self.focusable
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SizeWire {
    Columns(f64),
    Share(String),
}

impl SizeWire {
    fn value(self) -> Option<SizeValue> {
        match self {
            SizeWire::Columns(columns) => Some(SizeValue::Abs(columns as i64)),
            SizeWire::Share(share) => {
                let digits = share.strip_suffix('%')?;
                let (whole, fraction) = digits.split_once('.').unwrap_or((digits, "0"));
                let numeric =
                    |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
                (numeric(whole) && numeric(fraction))
                    .then(|| digits.parse().ok())
                    .flatten()
                    .map(SizeValue::Percent)
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum AnchorWire {
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    TopCenter,
    BottomCenter,
    LeftCenter,
    RightCenter,
}

impl From<AnchorWire> for OverlayAnchor {
    fn from(anchor: AnchorWire) -> Self {
        match anchor {
            AnchorWire::Center => OverlayAnchor::Center,
            AnchorWire::TopLeft => OverlayAnchor::TopLeft,
            AnchorWire::TopRight => OverlayAnchor::TopRight,
            AnchorWire::BottomLeft => OverlayAnchor::BottomLeft,
            AnchorWire::BottomRight => OverlayAnchor::BottomRight,
            AnchorWire::TopCenter => OverlayAnchor::TopCenter,
            AnchorWire::BottomCenter => OverlayAnchor::BottomCenter,
            AnchorWire::LeftCenter => OverlayAnchor::LeftCenter,
            AnchorWire::RightCenter => OverlayAnchor::RightCenter,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MarginWire {
    All(f64),
    Sides {
        #[serde(default)]
        top: f64,
        #[serde(default)]
        right: f64,
        #[serde(default)]
        bottom: f64,
        #[serde(default)]
        left: f64,
    },
}

impl From<MarginWire> for OverlayMargin {
    fn from(margin: MarginWire) -> Self {
        match margin {
            MarginWire::All(all) => OverlayMargin::all(all as i64),
            MarginWire::Sides {
                top,
                right,
                bottom,
                left,
            } => OverlayMargin {
                top: top as i64,
                right: right as i64,
                bottom: bottom as i64,
                left: left as i64,
            },
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayWire {
    width: Option<SizeWire>,
    min_width: Option<f64>,
    max_height: Option<SizeWire>,
    anchor: Option<AnchorWire>,
    offset_x: Option<f64>,
    offset_y: Option<f64>,
    row: Option<SizeWire>,
    col: Option<SizeWire>,
    margin: Option<MarginWire>,
    #[serde(default)]
    non_capturing: bool,
}

impl OverlayWire {
    pub fn options(self, visible: Rc<Cell<bool>>) -> OverlayOptions {
        OverlayOptions {
            width: self.width.and_then(SizeWire::value),
            min_width: self.min_width.map(|width| width as i64),
            max_height: self.max_height.and_then(SizeWire::value),
            anchor: self.anchor.map(OverlayAnchor::from),
            offset_x: self.offset_x.map(|x| x as i64),
            offset_y: self.offset_y.map(|y| y as i64),
            row: self.row.and_then(SizeWire::value),
            col: self.col.and_then(SizeWire::value),
            margin: self.margin.map(OverlayMargin::from),
            visible: Some(Rc::new(move |_, _| visible.get())),
            non_capturing: self.non_capturing,
        }
    }
}
