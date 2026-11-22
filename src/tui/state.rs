//! TUI data model: parsed `biketerra-ride` frames + 20Hz `window.btApp.riderController`
//! route geometry, local curved road ribbon, all-rider positions, draft & attack detection.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use serde_json::Value;

/// Ego samples kept for graphs (~5 min at 2Hz, same as the backend ring buffer).
const EGO_HISTORY: usize = 600;
/// Per-rider power samples kept for the spectate graph (~2 min at 2Hz).
const RIDER_HISTORY: usize = 240;

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64()).filter(|x| x.is_finite())
}

fn parse_sport(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) if n.as_i64() == Some(1) => 1,
        Some(Value::String(s)) => {
            let low = s.trim().to_ascii_lowercase();
            if low == "1" || low == "row" || low == "rowing" { 1 } else { 0 }
        }
        _ => 0,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Ego {
    pub speed: Option<f64>,
    pub speed_avg: Option<f64>,
    pub power: Option<f64>,
    pub power_avg: Option<f64>,
    pub wkg: Option<f64>,
    pub cadence: Option<f64>,
    pub cadence_avg: Option<f64>,
    pub hr: Option<f64>,
    pub hr_avg: Option<f64>,
    pub distance: Option<f64>,
    pub distance_remaining: Option<f64>,
    pub climb: Option<f64>,
    pub climb_remaining: Option<f64>,
    pub elevation: Option<f64>,
    pub grade: Option<f64>,
    pub draft: Option<f64>,
    pub lap: Option<f64>,
    pub sport: i64,
}

impl Ego {
    pub fn parse(v: &Value) -> Self {
        Self {
            speed: num(v, "speed"),
            speed_avg: num(v, "speedAvg"),
            power: num(v, "power"),
            power_avg: num(v, "powerAvg"),
            wkg: num(v, "wkg"),
            cadence: num(v, "cadence"),
            cadence_avg: num(v, "cadenceAvg"),
            hr: num(v, "heartRate"),
            hr_avg: num(v, "heartRateAvg"),
            distance: num(v, "distance"),
            distance_remaining: num(v, "distanceRemaining"),
            climb: num(v, "climb"),
            climb_remaining: num(v, "climbRemaining"),
            elevation: num(v, "elevation"),
            grade: num(v, "grade"),
            draft: num(v, "draftFactor"),
            lap: num(v, "lapCount"),
            sport: parse_sport(v.get("sport")),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Rider {
    pub id: String,
    pub name: String,
    pub is_ego: bool,
    pub is_focal: bool,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: Option<f64>,
    /// Heading-up local coordinates relative to focal rider centerline: right (m) and forward (m).
    pub rx: Option<f64>,
    pub rf: Option<f64>,
    /// Projected distance along the route polyline (metres).
    pub dist: Option<f64>,
    /// Distance delta relative to focal rider (metres, + ahead / - behind).
    pub dist_delta: Option<f64>,
    /// Route grade at rider's position (%).
    pub grade: Option<f64>,
    pub power: Option<f64>,
    pub wkg: Option<f64>,
    pub speed: Option<f64>,
    pub cadence: Option<f64>,
    pub hr: Option<f64>,
    pub gap: Option<f64>,
    pub lap: Option<f64>,
    pub draft: Option<f64>,
    pub sport: i64,
    pub color: Option<(u8, u8, u8)>,
    /// True when rider is surging / launching an attack.
    pub attacking: bool,
}

impl Rider {
    fn parse_broadcast(v: &Value) -> Self {
        let id = match v.get("id") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        Self {
            name: v.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
            id,
            power: num(v, "power"),
            wkg: num(v, "wkg"),
            speed: num(v, "speed"),
            cadence: num(v, "cadence"),
            hr: num(v, "heartRate"),
            gap: num(v, "gap"),
            lap: num(v, "lapCount"),
            sport: parse_sport(v.get("sport")),
            ..Default::default()
        }
    }

    fn parse_world(v: &Value) -> Self {
        let id = match v.get("id") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        let color = v.get("color").and_then(|c| c.as_array()).and_then(|a| {
            if a.len() >= 3 {
                Some((a[0].as_u64()? as u8, a[1].as_u64()? as u8, a[2].as_u64()? as u8))
            } else {
                None
            }
        });
        Self {
            id,
            name: v.get("name").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
            is_ego: v.get("isEgo").and_then(|x| x.as_bool()).unwrap_or(false),
            is_focal: v.get("isFocal").and_then(|x| x.as_bool()).unwrap_or(false),
            x: num(v, "x"),
            y: num(v, "y"),
            z: num(v, "z"),
            rx: num(v, "rx"),
            rf: num(v, "rf"),
            dist: num(v, "dist"),
            dist_delta: num(v, "distDelta"),
            grade: num(v, "grade"),
            power: num(v, "power"),
            wkg: num(v, "wkg"),
            speed: num(v, "speed"),
            cadence: num(v, "cadence"),
            hr: num(v, "hr"),
            gap: num(v, "gap"),
            lap: num(v, "lap"),
            draft: num(v, "draft"),
            sport: parse_sport(v.get("sport")),
            color,
            attacking: false,
        }
    }

    pub fn draft_pct(&self) -> Option<f64> {
        self.draft.map(|d| ((1.0 - d) * 100.0).clamp(0.0, 100.0))
    }
}

/// One sample of the high-res local curved road ribbon in heading-up focal coordinates.
#[derive(Clone, Copy, Debug)]
pub struct RoadSample {
    /// Centerline right offset (m) and forward offset (m) in focal heading frame.
    pub cx: f64,
    pub cf: f64,
    /// Unit right-hand road normal `(nr, nf)` in focal heading frame.
    pub nr: f64,
    pub nf: f64,
    /// Grade at this road slice (%).
    pub grade: f64,
    /// Signed route distance relative to focal rider (m).
    pub rel_dist: f64,
}

impl RoadSample {
    fn parse(v: &Value) -> Option<Self> {
        let a = v.as_array()?;
        if a.len() < 6 {
            return None;
        }
        Some(Self {
            cx: a[0].as_f64()?,
            cf: a[1].as_f64()?,
            nr: a[2].as_f64()?,
            nf: a[3].as_f64()?,
            grade: a[4].as_f64()?,
            rel_dist: a[5].as_f64()?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct RoutePt {
    pub dist: f64,
    pub x: f64,
    pub z: f64,
    pub elev: f64,
    pub grade: f64,
}

#[derive(Clone, Debug)]
pub struct RouteData {
    pub name: String,
    pub total_len: f64,
    pub is_loop: bool,
    pub min_elev: f64,
    pub max_elev: f64,
    pub total_climb: f64,
    pub points: Vec<RoutePt>,
}

impl RouteData {
    pub fn parse(v: &Value) -> Option<Self> {
        let arr = v.get("points")?.as_array()?;
        if arr.len() < 2 {
            return None;
        }
        let mut points = Vec::with_capacity(arr.len());
        for p in arr {
            let a = p.as_array()?;
            if a.len() < 5 {
                continue;
            }
            points.push(RoutePt {
                dist: a[0].as_f64()?,
                x: a[1].as_f64()?,
                z: a[2].as_f64()?,
                elev: a[3].as_f64()?,
                grade: a[4].as_f64()?,
            });
        }
        if points.len() < 2 {
            return None;
        }
        let total_len = num(v, "totalLen").unwrap_or_else(|| points.last().map(|p| p.dist).unwrap_or(1.0));
        let min_elev = num(v, "minElev").unwrap_or_else(|| points.iter().map(|p| p.elev).fold(f64::MAX, f64::min));
        let max_elev = num(v, "maxElev").unwrap_or_else(|| points.iter().map(|p| p.elev).fold(f64::MIN, f64::max));
        Some(Self {
            name: v.get("name").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            total_len,
            is_loop: v.get("isLoop").and_then(|b| b.as_bool()).unwrap_or(false),
            min_elev,
            max_elev,
            total_climb: num(v, "totalClimb").unwrap_or(0.0),
            points,
        })
    }

    /// Interpolate `(elevation_m, grade_pct)` at route distance `d` (m).
    pub fn sample_at(&self, d: f64) -> (f64, f64) {
        let d = d.clamp(0.0, self.total_len);
        let idx = self.points.partition_point(|p| p.dist < d);
        if idx == 0 {
            return (self.points[0].elev, self.points[0].grade);
        }
        if idx >= self.points.len() {
            let last = self.points.last().unwrap();
            return (last.elev, last.grade);
        }
        let a = &self.points[idx - 1];
        let b = &self.points[idx];
        let span = b.dist - a.dist;
        if span <= 1e-6 {
            return (a.elev, a.grade);
        }
        let t = ((d - a.dist) / span).clamp(0.0, 1.0);
        (a.elev + (b.elev - a.elev) * t, a.grade + (b.grade - a.grade) * t)
    }

    /// Interpolate elevation (m) at route distance `d` (m).
    pub fn elev_at(&self, d: f64) -> f64 {
        self.sample_at(d).0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Follow the page URL (ride / spectate / idle).
    Auto,
    Ride,
    Spectate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ride,
    Spectate,
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    Auto,
    Off,
    OneFps,
    Full,
}

impl RenderMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
            Self::OneFps => "1fps",
            Self::Full => "full",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "off" => Some(Self::Off),
            "1fps" => Some(Self::OneFps),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Off,
            Self::Off => Self::OneFps,
            Self::OneFps => Self::Full,
            Self::Full => Self::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoadViewMode {
    Road25,
    Road50,
    Road100,
    FullMap,
}

impl RoadViewMode {
    pub fn next(self) -> Self {
        match self {
            Self::Road25 => Self::Road50,
            Self::Road50 => Self::Road100,
            Self::Road100 => Self::FullMap,
            Self::FullMap => Self::Road25,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Road25 => "road ±25m",
            Self::Road50 => "road ±50m",
            Self::Road100 => "road ±100m",
            Self::FullMap => "full route map",
        }
    }
}

#[derive(Clone, Debug)]
struct SurgeTrack {
    fast_wkg: f64,
    slow_wkg: f64,
    last_at: Instant,
}

pub struct App {
    pub connected: bool,
    pub status: String,
    pub backend_version: Option<String>,
    pub url: String,
    pub ego: Option<Ego>,
    /// Broadcast riders (2Hz fallback when btApp isn't ready).
    pub broadcast_riders: Vec<Rider>,
    /// World riders from `window.btApp.riderController` (~20Hz, includes all riders + road coords).
    pub world_riders: Vec<Rider>,
    /// Active rider list shown in UI (world riders if available, else broadcast riders).
    pub riders: Vec<Rider>,
    pub route: Option<RouteData>,
    /// Local curved road ribbon around focal rider (±120 m, heading-up).
    pub road: Vec<RoadSample>,
    pub bt_app_ready: bool,
    pub window_hidden: bool,
    pub render_mode: RenderMode,
    pub effective_render: String,
    pub canvas_w: u32,
    pub canvas_h: u32,
    pub road_view: RoadViewMode,
    pub last_frame: Option<Instant>,
    pub world_ticks: VecDeque<Instant>,
    pub frames: u64,
    pub ego_hist: VecDeque<Ego>,
    pub rider_hist: HashMap<String, VecDeque<f64>>,
    surge_map: HashMap<String, SurgeTrack>,
    /// Id of the rider highlighted in the spectate list (kept stable when the order changes).
    pub selected_id: Option<String>,
    pub screen: Screen,
    pub show_help: bool,
    pub toast: Option<(String, Instant)>,
    pub quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            connected: false,
            status: "connecting…".into(),
            backend_version: None,
            url: String::new(),
            ego: None,
            broadcast_riders: Vec::new(),
            world_riders: Vec::new(),
            riders: Vec::new(),
            route: None,
            road: Vec::new(),
            bt_app_ready: false,
            window_hidden: false,
            render_mode: RenderMode::Auto,
            effective_render: "full".into(),
            canvas_w: 0,
            canvas_h: 0,
            road_view: RoadViewMode::Road50,
            last_frame: None,
            world_ticks: VecDeque::new(),
            frames: 0,
            ego_hist: VecDeque::new(),
            rider_hist: HashMap::new(),
            surge_map: HashMap::new(),
            selected_id: None,
            screen: Screen::Auto,
            show_help: false,
            toast: None,
            quit: false,
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    /// Measured world update rate (Hz) over the last 1.5 seconds.
    pub fn world_hz(&self) -> f64 {
        let now = Instant::now();
        let count = self
            .world_ticks
            .iter()
            .filter(|t| now.duration_since(**t).as_secs_f64() <= 1.5)
            .count();
        count as f64 / 1.5
    }

    /// The URL path (+query), e.g. `/ride?spectate=1`.
    pub fn path(&self) -> &str {
        let u = self.url.as_str();
        let rest = u.split_once("://").map(|(_, r)| r).unwrap_or(u);
        match rest.find('/') {
            Some(i) => &rest[i..],
            None => "/",
        }
    }

    /// What the page is doing according to its URL (falls back to "frames are arriving").
    pub fn page_mode(&self) -> Mode {
        let p = self.path();
        if self.url.is_empty() {
            return if self.frame_fresh() { Mode::Ride } else { Mode::Idle };
        }
        if p.starts_with("/spectate") || (p.starts_with("/ride") && p.contains("spectate=1")) {
            Mode::Spectate
        } else if p.starts_with("/ride") {
            Mode::Ride
        } else {
            Mode::Idle
        }
    }

    /// The screen to draw (manual override wins over the URL).
    pub fn mode(&self) -> Mode {
        match self.screen {
            Screen::Auto => self.page_mode(),
            Screen::Ride => Mode::Ride,
            Screen::Spectate => Mode::Spectate,
        }
    }

    pub fn frame_age_secs(&self) -> Option<f64> {
        self.last_frame.map(|t| t.elapsed().as_secs_f64())
    }

    pub fn frame_fresh(&self) -> bool {
        self.frame_age_secs().is_some_and(|a| a < 5.0)
    }

    /// Handle one JSON line from the backend socket.
    pub fn on_message(&mut self, msg: &Value) {
        match msg.get("event").and_then(|e| e.as_str()) {
            Some("hello") => {
                self.backend_version = msg.get("version").and_then(|v| v.as_str()).map(String::from);
                self.url = msg.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                self.ego_hist.clear();
                if let Some(hist) = msg.get("history").and_then(|h| h.as_array()) {
                    for f in hist {
                        if let Some(e) = f.get("ego").filter(|e| e.is_object()) {
                            self.push_ego(Ego::parse(e));
                        }
                    }
                }
                if let Some(h) = msg.get("host").filter(|x| x.is_object()) {
                    self.apply_host(h);
                }
                if let Some(r) = msg.get("route").filter(|x| x.is_object()) {
                    self.route = RouteData::parse(r);
                }
                if let Some(latest) = msg.get("latest").filter(|l| l.is_object()) {
                    self.apply_frame(latest, false);
                    self.last_frame = None;
                }
                if let Some(w) = msg.get("world").filter(|x| x.is_object()) {
                    self.apply_world(w, false);
                }
            }
            Some("ride") => {
                if let Some(data) = msg.get("data").filter(|d| d.is_object()) {
                    self.apply_frame(data, true);
                }
            }
            Some("route") => {
                if let Some(data) = msg.get("data").filter(|d| d.is_object()) {
                    self.route = RouteData::parse(data);
                }
            }
            Some("world") => {
                if let Some(data) = msg.get("data").filter(|d| d.is_object()) {
                    self.apply_world(data, true);
                }
            }
            Some("host") => {
                if let Some(data) = msg.get("data").filter(|d| d.is_object()) {
                    self.apply_host(data);
                }
            }
            Some("page") => {
                self.url = msg.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !self.url.contains("/ride") && !self.url.contains("/spectate") {
                    self.route = None;
                    self.road.clear();
                    self.world_riders.clear();
                    self.bt_app_ready = false;
                    self.rebuild_riders();
                }
            }
            _ => {}
        }
    }

    fn apply_host(&mut self, h: &Value) {
        if let Some(b) = h.get("windowHidden").and_then(|v| v.as_bool()) {
            self.window_hidden = b;
        }
        if let Some(m) = h.get("renderMode").and_then(|v| v.as_str()).and_then(RenderMode::from_str) {
            self.render_mode = m;
        }
    }

    fn push_ego(&mut self, e: Ego) {
        self.ego_hist.push_back(e);
        while self.ego_hist.len() > EGO_HISTORY {
            self.ego_hist.pop_front();
        }
    }

    fn apply_frame(&mut self, frame: &Value, live: bool) {
        let top_sport = parse_sport(frame.get("sport"));
        if let Some(e) = frame.get("ego").filter(|e| e.is_object()) {
            let mut ego = Ego::parse(e);
            if ego.sport == 0 && top_sport == 1 {
                ego.sport = 1;
            }
            if live {
                self.push_ego(ego.clone());
            }
            self.ego = Some(ego);
        }
        let riders: Vec<Rider> = frame
            .get("riders")
            .and_then(|r| r.as_array())
            .map(|a| a.iter().map(Rider::parse_broadcast).collect())
            .unwrap_or_default();
        self.broadcast_riders = riders;
        if live {
            self.frames += 1;
            self.last_frame = Some(Instant::now());
        }
        self.rebuild_riders();
        if live {
            self.record_rider_history();
        }
    }

    fn apply_world(&mut self, w: &Value, live: bool) {
        self.bt_app_ready = w.get("btAppReady").and_then(|v| v.as_bool()).unwrap_or(false);
        if let Some(b) = w.get("windowHidden").and_then(|v| v.as_bool()) {
            self.window_hidden = b;
        }
        if let Some(m) = w.get("renderMode").and_then(|v| v.as_str()).and_then(RenderMode::from_str) {
            self.render_mode = m;
        }
        if let Some(eff) = w.get("effectiveRender").and_then(|v| v.as_str()) {
            self.effective_render = eff.to_string();
        }
        if let Some(cw) = w.get("canvasW").and_then(|v| v.as_u64()) {
            self.canvas_w = cw as u32;
        }
        if let Some(ch) = w.get("canvasH").and_then(|v| v.as_u64()) {
            self.canvas_h = ch as u32;
        }
        if let Some(road_arr) = w.get("road").and_then(|v| v.as_array()) {
            self.road = road_arr.iter().filter_map(RoadSample::parse).collect();
        } else if !self.bt_app_ready {
            self.road.clear();
        }

        let mut riders: Vec<Rider> = w
            .get("riders")
            .and_then(|r| r.as_array())
            .map(|a| a.iter().map(Rider::parse_world).collect())
            .unwrap_or_default();

        if live {
            let now = Instant::now();
            self.world_ticks.push_back(now);
            while self
                .world_ticks
                .front()
                .is_some_and(|t| now.duration_since(*t).as_secs_f64() > 2.0)
            {
                self.world_ticks.pop_front();
            }

            // Update per-rider attack/surge EMA detector
            for r in &mut riders {
                if let Some(wkg) = r.wkg {
                    let entry = self.surge_map.entry(r.id.clone()).or_insert(SurgeTrack {
                        fast_wkg: wkg,
                        slow_wkg: wkg,
                        last_at: now,
                    });
                    let dt = now.duration_since(entry.last_at).as_secs_f64().clamp(0.01, 2.0);
                    entry.last_at = now;
                    let a_fast = 1.0 - (-dt / 2.0).exp();
                    let a_slow = 1.0 - (-dt / 18.0).exp();
                    entry.fast_wkg += (wkg - entry.fast_wkg) * a_fast;
                    entry.slow_wkg += (wkg - entry.slow_wkg) * a_slow;
                    let pwr = r.power.unwrap_or(0.0);
                    let high_effort = entry.fast_wkg >= 5.0 || pwr >= 360.0;
                    let surging = (entry.fast_wkg - entry.slow_wkg) >= 1.05
                        || (entry.slow_wkg > 0.5 && entry.fast_wkg >= entry.slow_wkg * 1.28);
                    r.attacking = (high_effort && surging) || entry.fast_wkg >= 6.5;
                }
            }
            self.surge_map.retain(|id, _| riders.iter().any(|r| &r.id == id));
        }

        if !riders.is_empty() || !self.bt_app_ready {
            self.world_riders = riders;
        }

        if live && self.bt_app_ready && !self.world_riders.is_empty() {
            self.last_frame = Some(Instant::now());
            // Keep ego power/speed/draft/grade fresh at 20Hz from world ego/focal rider
            if let Some(wr) = self
                .world_riders
                .iter()
                .find(|r| r.is_ego)
                .or_else(|| self.world_riders.iter().find(|r| r.is_focal))
            {
                let ego = self.ego.get_or_insert_with(Ego::default);
                if wr.power.is_some() {
                    ego.power = wr.power;
                }
                if wr.wkg.is_some() {
                    ego.wkg = wr.wkg;
                }
                if wr.speed.is_some() {
                    ego.speed = wr.speed;
                }
                if wr.draft.is_some() {
                    ego.draft = wr.draft;
                }
                if wr.grade.is_some() {
                    ego.grade = wr.grade.map(|g| g / 100.0);
                }
                if wr.y.is_some() {
                    ego.elevation = wr.y;
                }
                if ego.distance.is_none() && wr.dist.is_some() {
                    ego.distance = wr.dist;
                }
            }
        }
        self.rebuild_riders();
        if live && self.broadcast_riders.is_empty() {
            self.record_rider_history();
        }
    }

    fn rebuild_riders(&mut self) {
        let mut list = if !self.world_riders.is_empty() {
            let mut merged = self.world_riders.clone();
            // Cross-check broadcast_riders + ego so rowing (sport=1) is never lost between streams
            let ego_row = self.ego.as_ref().is_some_and(|e| e.sport == 1);
            for r in &mut merged {
                if r.sport == 1 {
                    continue;
                }
                if r.is_ego && ego_row {
                    r.sport = 1;
                    continue;
                }
                let r_name = r.name.trim().to_ascii_lowercase();
                let is_row = self.broadcast_riders.iter().any(|br| {
                    br.sport == 1
                        && ((!r.id.is_empty() && br.id == r.id)
                            || (!r_name.is_empty() && br.name.trim().to_ascii_lowercase() == r_name))
                });
                if is_row {
                    r.sport = 1;
                }
            }
            merged
        } else {
            self.broadcast_riders.clone()
        };

        let has_dist = list.iter().any(|r| r.dist.is_some());
        if has_dist {
            list.sort_by(|a, b| {
                b.dist
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&a.dist.unwrap_or(f64::NEG_INFINITY))
            });
        } else {
            list.sort_by(|a, b| {
                b.gap
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&a.gap.unwrap_or(f64::NEG_INFINITY))
            });
        }

        self.riders = list;
        if self.selected_id.as_ref().map_or(true, |id| !self.riders.iter().any(|r| &r.id == id)) {
            self.selected_id = self
                .riders
                .iter()
                .find(|r| r.is_focal)
                .or_else(|| self.riders.first())
                .map(|r| r.id.clone());
        }
    }

    fn record_rider_history(&mut self) {
        for r in &self.riders {
            let h = self.rider_hist.entry(r.id.clone()).or_default();
            h.push_back(r.power.unwrap_or(0.0));
            while h.len() > RIDER_HISTORY {
                h.pop_front();
            }
        }
        let riders = &self.riders;
        self.rider_hist.retain(|id, _| riders.iter().any(|r| &r.id == id));
    }

    pub fn selected_index(&self) -> Option<usize> {
        let id = self.selected_id.as_ref()?;
        self.riders.iter().position(|r| &r.id == id)
    }

    pub fn selected_rider(&self) -> Option<&Rider> {
        self.selected_index().map(|i| &self.riders[i])
    }

    pub fn focal_or_ego_rider(&self) -> Option<&Rider> {
        self.riders
            .iter()
            .find(|r| r.is_focal)
            .or_else(|| self.riders.iter().find(|r| r.is_ego))
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.riders.is_empty() {
            return;
        }
        let cur = self.selected_index().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.riders.len() as isize - 1) as usize;
        self.selected_id = Some(self.riders[next].id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(power: f64) -> Value {
        json!({
            "ego": { "speed": 10.0, "power": power, "distance": 1500.0, "grade": 0.05, "sport": 0 },
            "riders": [
                { "id": 1, "name": "behind", "power": 200, "gap": -12.0 },
                { "id": "b", "name": "ahead", "power": 300, "gap": 8.0 }
            ],
            "timestamp": 1
        })
    }

    #[test]
    fn parses_frames_and_tracks_history() {
        let mut app = App::new();
        app.on_message(&json!({ "event": "hello", "url": "https://biketerra.com/ride?id=1", "latest": null, "history": [] }));
        assert_eq!(app.mode(), Mode::Ride);

        app.on_message(&json!({ "event": "ride", "data": frame(250.0) }));
        app.on_message(&json!({ "event": "ride", "data": frame(260.0) }));
        assert_eq!(app.ego.as_ref().unwrap().power, Some(260.0));
        assert_eq!(app.ego_hist.len(), 2);
        assert_eq!(app.riders[0].name, "ahead");
        assert_eq!(app.riders[1].id, "1");
        assert_eq!(app.rider_hist["b"].len(), 2);
        assert_eq!(app.selected_id.as_deref(), Some("b"));
        app.move_selection(1);
        assert_eq!(app.selected_rider().unwrap().name, "behind");
    }

    #[test]
    fn parses_route_road_and_world_riders() {
        let mut app = App::new();
        app.on_message(&json!({
            "event": "route",
            "data": {
                "name": "Alpine Loop",
                "totalLen": 10000.0,
                "isLoop": true,
                "minElev": 100.0,
                "maxElev": 350.0,
                "totalClimb": 250.0,
                "points": [[0.0, 0.0, 0.0, 100.0, 0.0], [5000.0, 1000.0, 500.0, 350.0, 5.0], [10000.0, 0.0, 0.0, 100.0, -5.0]]
            }
        }));
        let rt = app.route.as_ref().unwrap();
        assert_eq!(rt.name, "Alpine Loop");
        assert!((rt.elev_at(2500.0) - 225.0).abs() < 0.01);

        app.on_message(&json!({
            "event": "world",
            "data": {
                "btAppReady": true,
                "renderMode": "off",
                "effectiveRender": "off",
                "windowHidden": true,
                "canvasW": 140,
                "canvasH": 90,
                "road": [[0.0, -40.0, 1.0, 0.0, 2.0, -40.0], [0.0, 0.0, 1.0, 0.0, 3.0, 0.0], [2.0, 40.0, 0.99, -0.1, 4.0, 40.0]],
                "riders": [
                    { "id": "10", "name": "Leader", "dist": 4200.0, "x": 800.0, "y": 310.0, "z": 400.0, "rx": 1.2, "rf": 0.0, "draft": 0.72, "power": 320, "wkg": 4.5, "isFocal": true },
                    { "id": "20", "name": "Attacker", "dist": 4215.0, "x": 815.0, "y": 311.0, "z": 400.0, "rx": -1.8, "rf": 15.0, "power": 520, "wkg": 7.2 }
                ]
            }
        }));
        // Broadcast says rider 20 is rowing (sport=1) while world packet omits sport
        app.on_message(&json!({
            "event": "ride",
            "data": {
                "ego": { "speed": 9.0, "power": 250, "sport": 0 },
                "riders": [
                    { "id": 10, "name": "Leader", "sport": 0 },
                    { "id": 20, "name": "Attacker", "sport": 1 }
                ]
            }
        }));
        assert!(app.bt_app_ready);
        assert_eq!(app.road.len(), 3);
        assert_eq!(app.riders[0].name, "Attacker");
        assert!(app.riders[0].attacking);
        assert_eq!(app.riders[0].sport, 1);
        assert_eq!(app.riders[1].sport, 0);
        assert!((app.riders[1].draft_pct().unwrap() - 28.0).abs() < 0.1);
    }

    #[test]
    fn detects_spectate_from_url() {
        let mut app = App::new();
        app.on_message(&json!({ "event": "page", "url": "https://biketerra.com/ride?id=3&spectate=1" }));
        assert_eq!(app.mode(), Mode::Spectate);
        app.on_message(&json!({ "event": "page", "url": "https://biketerra.com/spectate/abc" }));
        assert_eq!(app.mode(), Mode::Spectate);
        app.on_message(&json!({ "event": "page", "url": "https://biketerra.com/" }));
        assert_eq!(app.mode(), Mode::Idle);
        app.screen = Screen::Ride;
        assert_eq!(app.mode(), Mode::Ride);
    }
}
