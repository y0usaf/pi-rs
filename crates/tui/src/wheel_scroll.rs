const BURST_GAP_MS: f64 = 5.0;
const GESTURE_GAP_MS: f64 = 200.0;
const REFERENCE_GAP_MS: f64 = 100.0;
const MAX_AUTO_LINES: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WheelScrollLines {
    Auto,
    Lines(f64),
}

fn terminal_accelerates_wheel() -> bool {
    cfg!(target_os = "macos")
        && ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
            .iter()
            .all(|name| std::env::var_os(name).is_none())
}

pub struct WheelScroll {
    lines: WheelScrollLines,
    accelerate: bool,
    last_time: Option<f64>,
    last_direction: i64,
    average_gap: Option<f64>,
    carry: f64,
}

impl WheelScroll {
    pub fn new(lines: WheelScrollLines) -> WheelScroll {
        WheelScroll {
            lines,
            accelerate: !terminal_accelerates_wheel(),
            last_time: None,
            last_direction: 0,
            average_gap: None,
            carry: 0.0,
        }
    }

    pub fn set_lines(&mut self, lines: WheelScrollLines) {
        self.lines = lines;
        self.last_time = None;
        self.last_direction = 0;
        self.average_gap = None;
        self.carry = 0.0;
    }

    pub fn next(&mut self, direction: i64, now_ms: f64) -> i64 {
        if let WheelScrollLines::Lines(lines) = self.lines {
            return if lines.is_finite() {
                (lines.floor() as i64).max(1)
            } else {
                1
            };
        }
        if !self.accelerate {
            return 1;
        }
        let gap = self.last_time.map_or(f64::INFINITY, |last| now_ms - last);
        let same_gesture = direction == self.last_direction && gap <= GESTURE_GAP_MS;
        self.last_time = Some(now_ms);
        self.last_direction = direction;
        if !same_gesture {
            self.average_gap = None;
            self.carry = 0.0;
            return 1;
        }
        if gap < BURST_GAP_MS {
            return 1;
        }
        let average = self
            .average_gap
            .map_or(gap, |average| (average + gap) / 2.0);
        self.average_gap = Some(average);
        let lines = (REFERENCE_GAP_MS / average).clamp(1.0, MAX_AUTO_LINES) + self.carry;
        let whole = lines.floor();
        self.carry = lines - whole;
        whole as i64
    }
}
