// A world of civilizations that plays itself in a small window. Nothing to click.
//
// ponytail: the save file is "<seed> <year>" - the sim is deterministic, so resuming
// is just a replay and there is no serializer. Time away comes from the file's mtime.

use minifb::{Key, KeyRepeat, Scale, ScaleMode, Window, WindowOptions};
use std::time::{Duration, Instant, SystemTime};

const W: usize = 90; // world cells
const H: usize = 66;
const PX: usize = 8; // pixels per cell
const WIN_W: usize = 960;
const WIN_H: usize = 640;
const MAP_X: usize = 10;
const MAP_Y: usize = 10;
const PANEL_X: usize = MAP_X + W * PX + 10; // 740
const LOG_Y: usize = MAP_Y + H * PX + 10; // 548

const SAVE: &str = "civ.save";
const CFG: &str = "civ.cfg";
const DEFAULT_CFG: &str = "\
# idle civilizations. delete this file to get the defaults back.
speed 200        # milliseconds per year, lower is faster (+ and - change it live)
scale 1          # window size: 1, 2 or 4, clamped to what fits your screen
borderless 0     # 1 = no title bar and no frame. esc quits, there is no close button
topmost 0        # 1 = keep above other windows. with borderless 1 this covers things up
";
const MAX_OFFLINE: u32 = 5_000; // the world only ages so much while you're gone
/// The size at which a realm's reach peaks and rebellion starts to bite. Everything
/// about the balance keys off this, so it scales with the map instead of the constants.
const SPAN: f64 = 800.0;

const BG: u32 = 0x0E1116;
const INK: u32 = 0xC8D2E0;
const DIM: u32 = 0x63707F;
const LINE: u32 = 0x1E2430;

const PAL: [u32; 12] = [
    0xE05B5B, 0x5B9BE0, 0xE8C05A, 0x7BC96F, 0xB07BE0, 0x5AC8C8, 0xE8905A, 0xC8C8D2, 0xE07BA8,
    0xA8C85A, 0x8C8FE8, 0x4E9E7A,
];

const ERAS: [&str; 12] = [
    "Fire", "Pottery", "Bronze", "Writing", "Iron", "the Sail", "Gunpowder", "the Press", "Steam",
    "Electricity", "Flight", "Computing",
];

const SYL: [&str; 20] = [
    "ka", "zo", "ru", "mel", "tar", "vin", "osh", "bru", "ny", "sel", "dor", "ith", "aq", "umb",
    "ker", "lys", "gan", "tho", "ish", "vel",
];

struct Rng(u64);
impl Rng {
    fn u(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn f(&mut self) -> f64 {
        (self.u() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn n(&mut self, m: usize) -> usize {
        (self.u() % m as u64) as usize
    }
}

struct Tribe {
    name: String,
    pop: f64,
    tech: f64,
    land: u32,
    born: u32,
    alive: bool,
    col: usize,
}

impl Tribe {
    /// Force it can bring to one border cell: everything it has, spread along a
    /// border that grows as sqrt(land), then divided by overextension.
    ///
    /// Both terms earn their keep. Without the sqrt, big beats small so hard that one
    /// empire owns the map by year 25k and history stops. Without overextension the
    /// same thing happens later. And the penalty has to be quadratic, negligible below
    /// a third of SPAN: make it linear and shrinking makes a realm *stronger*, so every
    /// world freezes into a crystal of equal, immortal statelets.
    fn strength(&self) -> f64 {
        let land = self.land.max(1) as f64;
        self.pop * (1.0 + self.tech) / land.sqrt() / (1.0 + (land / SPAN).powi(2))
    }
}

struct World {
    fert: Vec<u8>,   // 0..=2 ocean depth, 3..=9 land fertility
    owner: Vec<u16>, // 0 = unclaimed, else tribe index + 1
    tribes: Vec<Tribe>,
    log: Vec<(u32, String)>,
    year: u32,
    rng: Rng,
}

impl World {
    fn new(seed: u64) -> World {
        let mut rng = Rng(seed | 1);
        let fert = terrain(&mut rng);
        let mut w = World {
            fert,
            owner: vec![0; W * H],
            tribes: Vec::new(),
            log: Vec::new(),
            year: 0,
            rng,
        };
        for _ in 0..6 {
            w.found();
        }
        w
    }

    fn say(&mut self, msg: String) {
        self.log.push((self.year, msg));
        if self.log.len() > 40 {
            self.log.drain(..20);
        }
    }

    /// Settle a new people on a random empty land cell, if the world has any left.
    fn found(&mut self) {
        let mut at = None;
        for _ in 0..200 {
            let i = self.rng.n(W * H);
            if self.fert[i] >= 3 && self.owner[i] == 0 {
                at = Some(i);
                break;
            }
        }
        let Some(at) = at else { return };
        let where_ = match (at % W, at / W) {
            (x, _) if x < W / 3 => "the west",
            (x, _) if x > 2 * W / 3 => "the east",
            (_, y) if y < H / 2 => "the north",
            _ => "the south",
        };
        let i = self.found_at(at);
        let msg = format!("the {} arise in {where_}", self.tribes[i].name);
        self.say(msg);
    }

    /// Create a people holding exactly the given cell, whoever held it before, and
    /// return its index. Dead slots get reused so the roster stays short.
    fn found_at(&mut self, at: usize) -> usize {
        // Take the least-used colour, ties broken by a random starting offset, so
        // neighbours on the map stay tellable apart.
        let mut used = [0usize; PAL.len()];
        for t in self.tribes.iter().filter(|t| t.alive) {
            used[t.col] += 1;
        }
        let off = self.rng.n(PAL.len());
        let col = (0..PAL.len())
            .map(|c| (c + off) % PAL.len())
            .min_by_key(|&c| used[c])
            .unwrap();
        let t = Tribe {
            name: name(&mut self.rng),
            pop: 200.0,
            tech: 0.0,
            land: 1,
            born: self.year,
            alive: true,
            col,
        };
        let idx = match self.tribes.iter().position(|t| !t.alive) {
            Some(i) => {
                self.tribes[i] = t;
                i
            }
            None => {
                self.tribes.push(t);
                self.tribes.len() - 1
            }
        };
        let old = self.owner[at] as usize;
        if old > 0 {
            self.tribes[old - 1].land -= 1;
        }
        self.owner[at] = idx as u16 + 1;
        idx
    }

    fn kill(&mut self, i: usize, why: &str) {
        let (name, age) = (self.tribes[i].name.clone(), self.year - self.tribes[i].born);
        self.tribes[i].alive = false;
        for c in self.owner.iter_mut() {
            if *c == i as u16 + 1 {
                *c = 0;
            }
        }
        self.say(format!("the {name} are no more, {why} after {age} years"));
    }

    fn tick(&mut self) {
        self.year += 1;
        let n = self.tribes.len();
        let mut food = vec![0.0; n];
        let mut cells: Vec<Vec<u32>> = vec![Vec::new(); n];
        for t in self.tribes.iter_mut() {
            t.land = 0;
        }
        for i in 0..W * H {
            let o = self.owner[i] as usize;
            if o > 0 {
                self.tribes[o - 1].land += 1;
                food[o - 1] += self.fert[i] as f64;
                cells[o - 1].push(i as u32);
            }
        }

        // Growth, discovery, and the occasional bad century.
        let best = self
            .tribes
            .iter()
            .filter(|t| t.alive)
            .fold(0.0f64, |m, t| m.max(t.tech));
        for i in 0..n {
            if !self.tribes[i].alive {
                continue;
            }
            let (r1, r2, r3, r4) = (self.rng.f(), self.rng.f(), self.rng.f(), self.rng.f());
            let t = &mut self.tribes[i];
            // Diminishing returns matter: with tech linear here, one cell eventually
            // feeds millions, cornered peoples become unconquerable and history stops.
            let cap = (food[i] * (1.0 + t.tech.sqrt() * 0.6) * 40.0).max(1.0);
            // Clamped: a people that suddenly loses most of its land has pop far over
            // capacity, and the raw logistic step then swings the population negative.
            let growth = 0.025 * (1.0 - t.pop / cap);
            t.pop *= 1.0 + growth.clamp(-0.5, 0.5);
            let before = t.tech as usize;
            t.tech += t.pop.sqrt() * 3e-5 * r1;
            t.tech += (best - t.tech).max(0.0) * 0.0002; // ideas cross borders, slowly
            let (era, nm) = (t.tech as usize, t.name.clone());
            if era > before && era <= ERAS.len() {
                self.say(format!("the {nm} discover {}", ERAS[era - 1]));
            }
            if r2 < 0.0012 {
                self.tribes[i].pop *= 0.55;
                self.say(format!("plague sweeps the {nm}"));
            } else if r3 < 0.0008 {
                self.tribes[i].tech += 0.5;
                self.say(format!("a golden age among the {nm}"));
            }
            // Rebellion is a big-empire disease: rare for a kingdom, chronic for a giant.
            // The empty check matters: a rebel can land in a reused slot later in this
            // same loop, alive and holding land but with no start-of-tick cells to pick.
            let l = self.tribes[i].land as f64;
            if !cells[i].is_empty() && l > SPAN / 10.0 && r4 < (l / SPAN).powi(2) * 0.005 {
                self.schism(i, &cells[i]);
            }
        }

        // Expansion and war: a bigger realm has a longer border to push on.
        for i in 0..n {
            let tries = 2 + (self.tribes[i].land / 100).min(8);
            for _ in 0..tries {
                if !self.tribes[i].alive || cells[i].is_empty() {
                    continue;
                }
                let src = cells[i][self.rng.n(cells[i].len())] as usize;
                let (x, y) = (src % W, src / W);
                let (nx, ny) = match self.rng.n(4) {
                    0 => (x + 1, y),
                    1 => (x.wrapping_sub(1), y),
                    2 => (x, y + 1),
                    _ => (x, y.wrapping_sub(1)),
                };
                if nx >= W || ny >= H {
                    continue;
                }
                self.push(i, ny * W + nx);
            }
            // With the Sail, a people can land a colony on any empty shore. Coastal only:
            // landing anywhere leaves single foreign cells stranded inland, which reads
            // as speckle rather than as settlement.
            if self.tribes[i].alive && self.tribes[i].tech >= 6.0 && self.rng.f() < 0.05 {
                let dst = self.rng.n(W * H);
                let shore = neighbours(dst % W, dst / W).any(|j| self.fert[j] < 3);
                if self.fert[dst] >= 3 && self.owner[dst] == 0 && shore {
                    self.owner[dst] = i as u16 + 1;
                    self.tribes[i].land += 1;
                }
            }
        }

        for i in 0..n {
            if self.tribes[i].alive && self.tribes[i].pop < 50.0 {
                self.kill(i, "starved");
            }
        }
        let alive = self.tribes.iter().filter(|t| t.alive).count();
        if alive == 0 || (alive < 3 && self.rng.f() < 0.02) {
            self.found();
        }
    }

    /// Try to take one cell for tribe `i`. Wilderness needs spare people, a neighbour's
    /// land needs an edge in strength - defenders get the terrain.
    fn push(&mut self, i: usize, dst: usize) {
        if self.fert[dst] < 3 || self.owner[dst] as usize == i + 1 {
            return;
        }
        let att = self.tribes[i].strength();
        let o = self.owner[dst] as usize;
        if o == 0 {
            if self.tribes[i].pop / self.tribes[i].land as f64 > 100.0 && self.rng.f() < 0.35 {
                self.owner[dst] = i as u16 + 1;
                self.tribes[i].land += 1;
            }
            return;
        }
        // Squared, so a real edge decides the border instead of coin-flipping it.
        // Linear odds leave every frontier mushy and the map never consolidates.
        let def = self.tribes[o - 1].strength() * 1.6;
        if self.rng.f() >= att * att / (att * att + def * def) {
            return;
        }
        self.owner[dst] = i as u16 + 1;
        self.tribes[i].land += 1;
        self.tribes[i].pop *= 0.998;
        self.tribes[o - 1].land -= 1;
        self.tribes[o - 1].pop *= 0.985;
        if self.tribes[o - 1].land == 0 {
            let by = self.tribes[i].name.clone();
            self.kill(o - 1, &format!("conquered by the {by}"));
        }
    }

    /// A province revolts: everything within 6 cells of the spark becomes its own people.
    fn schism(&mut self, i: usize, cells: &[u32]) {
        let spark = cells[self.rng.n(cells.len())] as usize;
        let (sx, sy) = ((spark % W) as i32, (spark / W) as i32);
        let parent = self.tribes[i].name.clone();
        // Rebels take their land off the parent, never off the wilderness: once the map
        // fills up there is none, and going through found() aborted every rebellion.
        let new = self.found_at(spark);
        let mut took = 1;
        for &c in cells {
            let c = c as usize;
            let (x, y) = ((c % W) as i32, (c / W) as i32);
            if c != spark && (x - sx).abs() + (y - sy).abs() <= 6 {
                self.owner[c] = new as u16 + 1;
                self.tribes[i].land -= 1;
                took += 1;
            }
        }
        let share = took as f64 / (self.tribes[i].land + took) as f64;
        self.tribes[new].pop = self.tribes[i].pop * share;
        self.tribes[new].tech = self.tribes[i].tech;
        self.tribes[new].land = took;
        self.tribes[i].pop *= 1.0 - share;
        let rebel = self.tribes[new].name.clone();
        self.say(format!("the {rebel} break away from the {parent}"));
        if self.tribes[i].land == 0 {
            self.kill(i, "swallowed by the rebellion");
        }
    }
}

/// Sines of a few frequencies, with the map edges sinking into ocean.
fn terrain(rng: &mut Rng) -> Vec<u8> {
    let (a, b, c) = (rng.f() * 6.3, rng.f() * 6.3, rng.f() * 6.3);
    let mut f = vec![0u8; W * H];
    for y in 0..H {
        for x in 0..W {
            let (fx, fy) = (x as f64 * 0.84, y as f64);
            let v = (fx * 0.11 + a).sin() * 0.9
                + (fy * 0.17 + b).cos() * 0.8
                + ((fx + fy) * 0.06 + c).sin() * 0.7
                + ((fx - fy * 1.3) * 0.09).cos() * 0.5
                + ((fx * 0.31 + fy * 0.23) + a).sin() * 0.45; // breaks up the blob
            let ex = (x as f64 / W as f64 - 0.5).abs() * 2.0;
            let ey = (y as f64 / H as f64 - 0.5).abs() * 2.0;
            let h = v - (ex.powi(3) + ey.powi(3)) * 1.6;
            f[y * W + x] = ((h + 1.15) * 3.0).clamp(0.0, 9.49) as u8;
        }
    }
    f
}

fn name(rng: &mut Rng) -> String {
    let mut s = String::new();
    for _ in 0..2 + rng.n(2) {
        s.push_str(SYL[rng.n(SYL.len())]);
    }
    s[..1].to_uppercase() + &s[1..]
}

fn hum(v: f64) -> String {
    match v {
        v if v >= 1e9 => format!("{:.1}B", v / 1e9),
        v if v >= 1e6 => format!("{:.1}M", v / 1e6),
        v if v >= 1e3 => format!("{:.1}K", v / 1e3),
        v => format!("{v:.0}"),
    }
}

// ---------------------------------------------------------------- drawing

fn rgb(r: u32, g: u32, b: u32) -> u32 {
    (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
}

fn shade(c: u32, k: f64) -> u32 {
    let f = |sh: u32| ((((c >> sh) & 0xFF) as f64 * k) as u32).min(255) << sh;
    f(16) | f(8) | f(0)
}

fn rect(buf: &mut [u32], x: usize, y: usize, w: usize, h: usize, col: u32) {
    for yy in y..(y + h).min(WIN_H) {
        for xx in x..(x + w).min(WIN_W) {
            buf[yy * WIN_W + xx] = col;
        }
    }
}

/// 5x7 uppercase pixel font. Everything drawn gets upcased, which suits the look.
const GLYPHS: [[u8; 7]; 43] = [
    [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11], // A
    [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
    [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
    [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
    [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
    [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
    [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
    [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
    [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
    [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
    [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
    [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
    [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
    [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
    [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
    [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
    [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
    [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
    [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
    [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
    [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
    [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
    [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
    [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
    [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F], // Z
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E], // 0
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
    [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
    [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C], // 9
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // space
    [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00], // -
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C], // .
    [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08], // ,
    [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00], // :
    [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10], // /
    [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04], // !
];

fn glyph(c: char) -> &'static [u8; 7] {
    let i = match c.to_ascii_uppercase() {
        c @ 'A'..='Z' => c as usize - 'A' as usize,
        c @ '0'..='9' => 26 + c as usize - '0' as usize,
        '-' => 37,
        '.' => 38,
        ',' => 39,
        ':' => 40,
        '/' => 41,
        '!' => 42,
        _ => 36,
    };
    &GLYPHS[i]
}

/// Draws text and returns the x it ended at, so callers can chain segments.
fn text(buf: &mut [u32], x: usize, y: usize, s: &str, col: u32, scale: usize) -> usize {
    let mut cx = x;
    for ch in s.chars() {
        let g = glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            for bit in 0..5 {
                if bits & (0x10 >> bit) == 0 {
                    continue;
                }
                let (px, py) = (cx + bit * scale, y + row * scale);
                if px + scale <= WIN_W && py + scale <= WIN_H {
                    rect(buf, px, py, scale, scale, col);
                }
            }
        }
        cx += 6 * scale;
    }
    cx
}

fn render(w: &World, buf: &mut [u32]) {
    buf.fill(BG);

    for y in 0..H {
        for x in 0..W {
            let i = y * W + x;
            let f = w.fert[i] as u32;
            let o = w.owner[i] as usize;
            let col = if f < 3 {
                // Ocean, lighter where it laps against a coast.
                let base = rgb(9 + f * 3, 24 + f * 8, 44 + f * 14);
                if neighbours(x, y).any(|j| w.fert[j] >= 3) {
                    shade(base, 1.5)
                } else {
                    base
                }
            } else if o == 0 {
                let g = f - 3;
                rgb(44 + g * 3, 62 + g * 8, 38 + g * 3)
            } else {
                // Most land sits at the low end of the fertility range, so the floor
                // here is what sets the overall brightness, not the span.
                shade(PAL[w.tribes[o - 1].col], 0.75 + 0.042 * (f - 3) as f64)
            };
            // A darker edge wherever ownership changes: this is what makes it read as
            // a map of realms rather than a field of coloured dots.
            let right = x + 1 < W && w.owner[i + 1] != w.owner[i];
            let down = y + 1 < H && w.owner[i + W] != w.owner[i];
            let (px0, py0) = (MAP_X + x * PX, MAP_Y + y * PX);
            for dy in 0..PX {
                for dx in 0..PX {
                    let edge = (right && dx == PX - 1) || (down && dy == PX - 1);
                    buf[(py0 + dy) * WIN_W + px0 + dx] =
                        if edge { shade(col, 0.5) } else { col };
                }
            }
        }
    }

    // Panel: who is who, biggest first.
    let mut idx: Vec<usize> = (0..w.tribes.len()).filter(|&i| w.tribes[i].alive).collect();
    idx.sort_by(|&a, &b| w.tribes[b].land.cmp(&w.tribes[a].land));
    let px = PANEL_X;
    text(buf, px, MAP_Y, &format!("YEAR {}", w.year), INK, 2);
    text(
        buf,
        px,
        MAP_Y + 18,
        &format!("{} CIVILIZATIONS", idx.len()),
        DIM,
        1,
    );
    let widest = idx.first().map_or(1, |&i| w.tribes[i].land).max(1) as f64;
    for (row, &i) in idx.iter().take(11).enumerate() {
        let t = &w.tribes[i];
        let y = MAP_Y + 38 + row * 44;
        rect(buf, px, y, 8, 8, PAL[t.col]);
        text(buf, px + 13, y, &t.name, INK, 2);
        let bar = (t.land as f64 / widest * 196.0) as usize;
        rect(buf, px, y + 16, 196, 5, LINE);
        rect(buf, px, y + 16, bar.max(1), 5, shade(PAL[t.col], 0.85));
        text(
            buf,
            px,
            y + 25,
            &format!("{}  LAND {}  TECH {}", hum(t.pop), t.land, t.tech as u32),
            DIM,
            1,
        );
    }

    // Always on screen, because in borderless mode this is the only way out.
    text(
        buf,
        PANEL_X,
        WIN_H - 16,
        "SPACE PAUSE  +/- SPEED  ESC QUIT",
        shade(DIM, 0.8),
        1,
    );

    // Chronicle.
    rect(buf, MAP_X, LOG_Y - 6, WIN_W - 2 * MAP_X, 1, LINE);
    for (row, (year, msg)) in w.log.iter().rev().take(4).rev().enumerate() {
        let y = LOG_Y + row * 19;
        let after = text(buf, MAP_X, y, &format!("{year}"), shade(INK, 0.55), 2);
        text(buf, after + 12, y, msg, INK, 2);
    }
}

fn neighbours(x: usize, y: usize) -> impl Iterator<Item = usize> {
    [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .filter_map(move |(dx, dy)| {
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            (nx >= 0 && ny >= 0 && (nx as usize) < W && (ny as usize) < H)
                .then(|| ny as usize * W + nx as usize)
        })
}

/// Usable desktop, so the window can never be opened bigger than the screen.
#[cfg(windows)]
fn screen() -> (usize, usize) {
    unsafe extern "system" {
        fn GetSystemMetrics(i: i32) -> i32;
    }
    // SM_CXFULLSCREEN / SM_CYFULLSCREEN: the client area of a maximised window, so
    // the taskbar is already excluded.
    unsafe {
        (
            GetSystemMetrics(16).max(640) as usize,
            GetSystemMetrics(17).max(480) as usize,
        )
    }
}

#[cfg(not(windows))]
fn screen() -> (usize, usize) {
    (WIN_W, WIN_H)
}

/// 24-bit BMP, bottom-up. Lets you keep a picture of a world, and is how the look of
/// this thing gets checked without a human squinting at the window.
fn write_bmp(path: &str, buf: &[u32]) -> std::io::Result<()> {
    let row = WIN_W * 3; // 2880, already a multiple of 4, so no padding
    let size = 54 + row * WIN_H;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(WIN_W as i32).to_le_bytes());
    out.extend_from_slice(&(WIN_H as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for y in (0..WIN_H).rev() {
        for x in 0..WIN_W {
            let p = buf[y * WIN_W + x];
            out.extend_from_slice(&[p as u8, (p >> 8) as u8, (p >> 16) as u8]);
        }
    }
    std::fs::write(path, out)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let now = || {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0x9E37_79B9, |d| d.as_nanos() as u64)
    };

    // Settings come from civ.cfg, which is written out on first run so there is
    // something to find and edit. Command line flags override it.
    let cfg = std::fs::read_to_string(CFG).unwrap_or_else(|_| {
        let _ = std::fs::write(CFG, DEFAULT_CFG);
        DEFAULT_CFG.to_string()
    });
    let (mut speed, mut scale, mut borderless, mut topmost) = (200u64, "1".to_string(), false, false);
    for line in cfg.lines() {
        let mut it = line.split('#').next().unwrap_or("").split_whitespace();
        let yes = |v: &str| v == "1" || v == "true" || v == "yes";
        match (it.next(), it.next()) {
            (Some("speed"), Some(v)) => speed = v.parse().unwrap_or(speed),
            (Some("scale"), Some(v)) => scale = v.to_string(),
            (Some("borderless"), Some(v)) => borderless = yes(v),
            (Some("topmost"), Some(v)) => topmost = yes(v),
            _ => {}
        }
    }
    if let Some(v) = arg("--speed") {
        speed = v.parse().unwrap_or(speed);
    }
    if let Some(v) = arg("--scale") {
        scale = v;
    }
    borderless |= args.iter().any(|a| a == "--borderless");
    topmost |= args.iter().any(|a| a == "--topmost");
    let speed = speed.clamp(8, 5000);

    let saved = std::fs::read_to_string(SAVE).unwrap_or_default();
    let mut it = saved.split_whitespace();
    let (mut seed, mut year) = (
        it.next().and_then(|s| s.parse().ok()).unwrap_or_else(now),
        it.next().and_then(|s| s.parse().ok()).unwrap_or(0u32),
    );
    // Years that passed while the program wasn't running: that is the idle part.
    // Measured in configured years, so a faster game also ages faster while away.
    let away = std::fs::metadata(SAVE)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map_or(0, |d| (d.as_millis() / speed as u128) as u32)
        .min(MAX_OFFLINE);
    if args.iter().any(|a| a == "--fresh") {
        (seed, year) = (now(), 0);
    }

    let mut w = World::new(seed);
    for _ in 0..year + away {
        w.tick();
    }
    if away > 0 {
        w.say(format!("{away} years passed while you were away"));
    }

    let mut buf = vec![BG; WIN_W * WIN_H];
    if let Some(path) = arg("--shot") {
        w.tick();
        render(&w, &mut buf);
        write_bmp(&path, &buf).expect("write bmp");
        return;
    }

    // Clamped to what the screen can actually hold. An oversized always-on-top window
    // with no title bar has no close button and no edges to drag: it just eats the
    // desktop. Never let a settings value put the user in that position.
    let want: u32 = match scale.as_str() {
        "2" => 2,
        "4" => 4,
        _ => 1,
    };
    let (sw, sh) = screen();
    let fits = ((sw / WIN_W).min(sh / WIN_H) as u32).max(1);
    // ponytail: the world grid is fixed, so "window size" resizes the view, not the
    // map. Drag the frame or set scale; the picture stretches and keeps its aspect.
    let mut win = Window::new(
        "idle civilizations",
        WIN_W,
        WIN_H,
        WindowOptions {
            scale: match want.min(fits) {
                4 => Scale::X4,
                2 | 3 => Scale::X2,
                _ => Scale::X1,
            },
            scale_mode: ScaleMode::AspectRatioStretch,
            resize: !borderless,
            // `none` is the one that actually removes the chrome: minifb's `borderless`
            // only drops WS_THICKFRAME, and a zero window style is WS_OVERLAPPED, which
            // has a title bar by definition. Verified against the window's style bits.
            none: borderless,
            borderless,
            title: !borderless,
            topmost,
            ..WindowOptions::default()
        },
    )
    .expect("open window");

    let mut step = Duration::from_millis(speed); // one year
    let mut last = Instant::now();
    let mut paused = false;
    render(&w, &mut buf);
    while win.is_open() && !win.is_key_down(Key::Escape) {
        if win.is_key_pressed(Key::Space, KeyRepeat::No) {
            paused = !paused;
        }
        if win.is_key_pressed(Key::Equal, KeyRepeat::No) {
            step = (step / 2).max(Duration::from_millis(12));
        }
        if win.is_key_pressed(Key::Minus, KeyRepeat::No) {
            step = (step * 2).min(Duration::from_millis(1600));
        }
        if !paused && last.elapsed() >= step {
            last = Instant::now();
            w.tick();
            render(&w, &mut buf);
            if w.year % 25 == 0 {
                let _ = std::fs::write(SAVE, format!("{seed} {}", w.year));
            }
        }
        let _ = win.update_with_buffer(&buf, WIN_W, WIN_H);
        std::thread::sleep(Duration::from_millis(8));
    }
    let _ = std::fs::write(SAVE, format!("{seed} {}", w.year));
}

#[test]
fn history_happens_and_stays_sane() {
    let mut w = World::new(7);
    let land = w.fert.iter().filter(|&&f| f >= 3).count() as f64 / (W * H) as f64;
    assert!((0.2..0.85).contains(&land), "land fraction {land}");

    for _ in 0..3000 {
        w.tick();
        for i in 0..W * H {
            let o = w.owner[i] as usize;
            assert!(o <= w.tribes.len() && (o == 0 || w.tribes[o - 1].alive));
            assert!(o == 0 || w.fert[i] >= 3, "someone settled the ocean");
        }
    }
    assert!(w.tribes.iter().any(|t| t.alive), "world went extinct");
    for t in w.tribes.iter().filter(|t| t.alive) {
        assert!(t.land > 0);
    }
    assert!(w.log.len() > 3, "3000 years and nothing happened");

    // Both of these caught real bugs. A people that loses most of its land in one year
    // sits far over capacity, and the unclamped logistic step sent its population to
    // minus 28 million - which the starvation check then quietly reported as a famine.
    for _ in 3000..40000 {
        w.tick();
        for t in w.tribes.iter() {
            assert!(t.pop.is_finite() && t.pop > 0.0, "population went bad");
        }
    }
    // And history has to keep happening. Three separate tunings looked fine early on,
    // then locked into a crystal of immortal statelets that never changed again.
    let newest = w
        .tribes
        .iter()
        .filter(|t| t.alive)
        .map(|t| t.born)
        .max()
        .unwrap();
    assert!(
        newest > 20000,
        "world froze: newest living people dates to {newest}"
    );

    // Resuming is a replay, so the same seed must give the same history.
    let mut b = World::new(7);
    let mut c = World::new(7);
    for _ in 0..3000 {
        b.tick();
        c.tick();
    }
    assert_eq!(b.log, c.log);
    assert_eq!(b.owner, c.owner);
}
