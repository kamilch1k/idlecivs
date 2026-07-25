// A world of civilizations that plays itself in a small window. Nothing to click.
//
// ponytail: the save file is "<seed> <year>" - the sim is deterministic, so resuming
// is just a replay and there is no serializer. Time away comes from the file's mtime.

use minifb::{Key, KeyRepeat, Scale, Window, WindowOptions};
use std::time::{Duration, Instant, SystemTime};

const W: usize = 90; // world cells
const H: usize = 66;
const PX: usize = 8; // pixels per cell at the default zoom, where the whole world fits
const MAP_PW: usize = W * PX; // map viewport, 720 x 528
const MAP_PH: usize = H * PX;
const ZOOM_MIN: usize = PX; // below this the world would not cover the viewport
const ZOOM_MAX: usize = 24;
const WIN_W: usize = 960;
const WIN_H: usize = 640;
const MAP_X: usize = 10;
const MAP_Y: usize = 10;
const PANEL_X: usize = MAP_X + W * PX + 10; // 740
const LOG_Y: usize = MAP_Y + H * PX + 10; // 548

const SAVE: &str = "civ.save";
const CFG: &str = "civ.cfg";
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
    base: Vec<u32>,                         // ocean and wilderness, fixed for the world
    art: [[(u32, u32); 7]; PAL.len()],      // civ colour by fertility, lit and edge
}

impl World {
    fn new(seed: u64) -> World {
        let mut rng = Rng(seed | 1);
        let fert = terrain(&mut rng);
        let base = base_colors(&fert);
        let mut w = World {
            fert,
            owner: vec![0; W * H],
            tribes: Vec::new(),
            log: Vec::new(),
            year: 0,
            rng,
            base,
            art: tribe_colors(),
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
const GLYPHS: [[u8; 7]; 45] = [
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
    [0x10, 0x08, 0x04, 0x02, 0x04, 0x08, 0x10], // >
    [0x01, 0x02, 0x04, 0x08, 0x04, 0x02, 0x01], // <
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
        '>' => 43,
        '<' => 44,
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

/// Everything on the map that never changes: ocean depth with its coastal highlight,
/// and unclaimed land by fertility. Computed once, because at 8 pixels a cell a pan
/// touches 380k pixels and none of this is worth recomputing per frame.
fn base_colors(fert: &[u8]) -> Vec<u32> {
    (0..W * H)
        .map(|i| {
            let f = fert[i] as u32;
            if f < 3 {
                let base = rgb(9 + f * 3, 24 + f * 8, 44 + f * 14);
                if neighbours(i % W, i / W).any(|j| fert[j] >= 3) {
                    shade(base, 1.5)
                } else {
                    base
                }
            } else {
                let g = f - 3;
                rgb(44 + g * 3, 62 + g * 8, 38 + g * 3)
            }
        })
        .collect()
}

/// Every civ colour at every fertility, plus the darker edge variant. 12 x 7 entries,
/// so the inner pixel loop is two array lookups instead of floating point per pixel.
fn tribe_colors() -> [[(u32, u32); 7]; PAL.len()] {
    let mut t = [[(0, 0); 7]; PAL.len()];
    for (c, &p) in PAL.iter().enumerate() {
        for f in 0..7 {
            // Most land sits at the low end of the fertility range, so the floor here
            // is what sets the overall brightness, not the span.
            let lit = shade(p, 0.75 + 0.042 * f as f64);
            t[c][f] = (lit, shade(lit, 0.5));
        }
    }
    t
}

/// Fill in map viewport coordinates, clipped to it, so a cell straddling the edge of
/// the view draws only its visible part instead of bleeding over the panel.
fn vfill(buf: &mut [u32], vx: i32, vy: i32, vw: i32, vh: i32, col: u32) {
    let (x0, y0) = (vx.max(0), vy.max(0));
    let (x1, y1) = (
        (vx + vw).min(MAP_PW as i32),
        (vy + vh).min(MAP_PH as i32),
    );
    for y in y0..y1 {
        let row = (MAP_Y + y as usize) * WIN_W + MAP_X;
        for x in x0..x1 {
            buf[row + x as usize] = col;
        }
    }
}

fn clamp_cam(cam: (i32, i32), zoom: usize) -> (i32, i32) {
    let (mx, my) = (
        (W * zoom - MAP_PW) as i32,
        (H * zoom - MAP_PH) as i32,
    );
    (cam.0.clamp(0, mx), cam.1.clamp(0, my))
}

fn render(w: &World, buf: &mut [u32], cam: (i32, i32), zoom: usize) {
    buf.fill(BG);

    // Only the cells the viewport can see, drawn as clipped rectangles. Walking pixels
    // instead and dividing per pixel to find the cell was five times slower.
    let z = zoom as i32;
    let (cx0, cy0) = (cam.0 as usize / zoom, cam.1 as usize / zoom);
    let cx1 = ((cam.0 as usize + MAP_PW - 1) / zoom).min(W - 1);
    let cy1 = ((cam.1 as usize + MAP_PH - 1) / zoom).min(H - 1);
    for cy in cy0..=cy1 {
        for cx in cx0..=cx1 {
            let i = cy * W + cx;
            let o = w.owner[i] as usize;
            let (lit, dark) = if o == 0 {
                (w.base[i], shade(w.base[i], 0.5))
            } else {
                w.art[w.tribes[o - 1].col][(w.fert[i] - 3) as usize]
            };
            let (vx, vy) = ((cx * zoom) as i32 - cam.0, (cy * zoom) as i32 - cam.1);
            vfill(buf, vx, vy, z, z, lit);
            // A darker edge wherever ownership changes: this is what makes it read as
            // a map of realms rather than a field of coloured dots.
            if cx + 1 < W && w.owner[i + 1] as usize != o {
                vfill(buf, vx + z - 1, vy, 1, z, dark);
            }
            if cy + 1 < H && w.owner[i + W] as usize != o {
                vfill(buf, vx, vy + z - 1, z, 1, dark);
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
    let hint = shade(DIM, 0.8);
    text(buf, PANEL_X, WIN_H - 28, "DRAG PAN   WHEEL ZOOM", hint, 1);
    text(
        buf,
        PANEL_X,
        WIN_H - 16,
        "ESC SETTINGS  SPACE PAUSE  Q QUIT",
        hint,
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

#[derive(Clone, Copy, PartialEq, Debug)]
struct Settings {
    speed: u64, // milliseconds per year
    scale: u32,
    borderless: bool,
    topmost: bool,
}

impl Settings {
    const ROWS: usize = 4;

    /// One left/right press on a settings row. Right always means more of the thing:
    /// faster, bigger, on. Returns whether the window has to be reopened for it.
    fn adjust(&mut self, row: usize, d: i32) -> bool {
        match row {
            0 => {
                self.speed = if d > 0 {
                    (self.speed / 2).max(8)
                } else {
                    (self.speed * 2).min(5000)
                };
                false
            }
            1 => {
                // 3 is skipped: minifb has no X3, so it would silently behave as X2.
                self.scale = match (self.scale, d > 0) {
                    (1, true) => 2,
                    (2, true) => 4,
                    (4, false) => 2,
                    (2, false) => 1,
                    (s, _) => s,
                };
                true
            }
            2 => {
                self.borderless = !self.borderless;
                true
            }
            _ => {
                self.topmost = !self.topmost;
                false // applied live, no new window needed
            }
        }
    }
}

/// The settings file, also what the settings screen writes back.
fn cfg_text(s: &Settings) -> String {
    let b = |v: bool| if v { 1 } else { 0 };
    format!(
        "# idle civilizations. delete this file to get the defaults back.
# esc opens the settings screen in the game and writes this file for you.
speed {}        # milliseconds per year, lower is faster
scale {}          # window size: 1, 2 or 4, clamped to what fits your screen
borderless {}     # 1 = no title bar and no frame
topmost {}        # 1 = keep above other windows
",
        s.speed,
        s.scale,
        b(s.borderless),
        b(s.topmost)
    )
}

fn parse_cfg(text: &str, s: &mut Settings) {
    let yes = |v: &str| v == "1" || v == "true" || v == "yes";
    for line in text.lines() {
        let mut it = line.split('#').next().unwrap_or("").split_whitespace();
        match (it.next(), it.next()) {
            (Some("speed"), Some(v)) => s.speed = v.parse().unwrap_or(s.speed),
            (Some("scale"), Some(v)) => s.scale = v.parse().unwrap_or(s.scale),
            (Some("borderless"), Some(v)) => s.borderless = yes(v),
            (Some("topmost"), Some(v)) => s.topmost = yes(v),
            _ => {}
        }
    }
}

/// Opens the window, never bigger than the screen. An oversized always-on-top window
/// with no title bar has no close button and no edges to drag: it just eats the desktop,
/// so no settings value is allowed to put the user in that position.
///
/// `none` is the option that actually removes the chrome: minifb's `borderless` only
/// drops WS_THICKFRAME, and a zero window style is WS_OVERLAPPED, which has a title bar
/// by definition. Checked against the real window's style bits.
fn open_window(s: &Settings) -> Window {
    let (sw, sh) = screen();
    let fits = ((sw / WIN_W).min(sh / WIN_H) as u32).max(1);
    Window::new(
        "idle civilizations",
        WIN_W,
        WIN_H,
        WindowOptions {
            scale: match s.scale.min(fits) {
                4 => Scale::X4,
                2 | 3 => Scale::X2,
                _ => Scale::X1,
            },
            // Resize stays off on purpose: minifb scales mouse positions by the integer
            // scale factor only, with a "needs to be fixed with resize support" note in
            // its source, so a drag-resized window puts every click in the wrong place.
            // Window size comes from the scale setting instead.
            resize: false,
            none: s.borderless,
            borderless: s.borderless,
            title: !s.borderless,
            topmost: s.topmost,
            ..WindowOptions::default()
        },
    )
    .expect("open window")
}

const MENU_W: usize = 420;
const MENU_H: usize = 214;
/// Shared by the drawing and the clicking, so they cannot disagree about where a row is.
fn menu_at() -> (usize, usize) {
    (
        MAP_X + (MAP_PW - MENU_W) / 2,
        MAP_Y + (MAP_PH - MENU_H) / 2,
    )
}
fn menu_row_y(row: usize) -> usize {
    menu_at().1 + 52 + row * 26
}

/// Which row and which direction a click at buffer position (x, y) means. `None` when
/// the click is outside the box, which the caller treats as closing the menu.
fn menu_hit(x: usize, y: usize) -> Option<(usize, i32)> {
    let (bx, by) = menu_at();
    if x < bx || y < by || x >= bx + MENU_W || y >= by + MENU_H {
        return None;
    }
    for row in 0..Settings::ROWS {
        let ry = menu_row_y(row);
        if y + 6 >= ry && y < ry + 20 {
            let d = if (bx + 216..bx + 244).contains(&x) {
                -1
            } else if (bx + 372..bx + 400).contains(&x) {
                1
            } else {
                0
            };
            return Some((row, d));
        }
    }
    Some((usize::MAX, 0)) // inside the box but not on a row: swallow the click
}

fn draw_menu(buf: &mut [u32], sel: usize, s: &Settings) {
    let (bw, bh) = (MENU_W, MENU_H);
    let (x, y) = menu_at();
    rect(buf, x, y, bw, bh, 0x121822);
    rect(buf, x, y, bw, 1, INK);
    rect(buf, x, y + bh - 1, bw, 1, INK);
    rect(buf, x, y, 1, bh, INK);
    rect(buf, x + bw - 1, y, 1, bh, INK);
    text(buf, x + 20, y + 16, "SETTINGS", INK, 2);

    let on_off = |v: bool| if v { "ON" } else { "OFF" };
    let rows = [
        ("SPEED", format!("{} MS/YEAR", s.speed)),
        ("WINDOW", format!("{}X", s.scale)),
        ("TITLE BAR", on_off(!s.borderless).to_string()),
        ("ALWAYS ON TOP", on_off(s.topmost).to_string()),
    ];
    for (i, (k, v)) in rows.iter().enumerate() {
        let ry = menu_row_y(i);
        let hot = i == sel;
        let col = if hot { 0xE8C05A } else { INK };
        if hot {
            text(buf, x + 20, ry, ">", col, 2);
        }
        text(buf, x + 40, ry, k, col, 2);
        // Clickable arrows, at the coordinates menu_hit tests against.
        text(buf, x + 220, ry, "<", if hot { col } else { DIM }, 2);
        text(buf, x + 246, ry, v, if hot { col } else { DIM }, 2);
        text(buf, x + 376, ry, ">", if hot { col } else { DIM }, 2);
    }
    text(buf, x + 20, y + bh - 40, "CLICK THE ARROWS, OR UP DOWN LEFT RIGHT", DIM, 1);
    text(buf, x + 20, y + bh - 26, "ESC OR CLICK OUTSIDE TO CLOSE AND SAVE", DIM, 1);
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

/// Cursor and window position in screen pixels. minifb can set a window position but
/// not read one, and its mouse position is client relative, so moving a frameless
/// window by dragging needs both from the OS or the window chases the cursor.
#[cfg(windows)]
fn cursor_and_window(hwnd: *mut std::ffi::c_void) -> ((i32, i32), (i32, i32)) {
    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct Rect {
        l: i32,
        t: i32,
        r: i32,
        b: i32,
    }
    unsafe extern "system" {
        fn GetCursorPos(p: *mut Point) -> i32;
        fn GetWindowRect(h: *mut std::ffi::c_void, r: *mut Rect) -> i32;
    }
    unsafe {
        let mut p = Point { x: 0, y: 0 };
        let mut r = Rect {
            l: 0,
            t: 0,
            r: 0,
            b: 0,
        };
        GetCursorPos(&mut p);
        GetWindowRect(hwnd, &mut r);
        ((p.x, p.y), (r.l, r.t))
    }
}

#[cfg(not(windows))]
fn cursor_and_window(_hwnd: *mut std::ffi::c_void) -> ((i32, i32), (i32, i32)) {
    ((0, 0), (0, 0))
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
    let mut set = Settings {
        speed: 200,
        scale: 1,
        borderless: false,
        topmost: false,
    };
    match std::fs::read_to_string(CFG) {
        Ok(t) => parse_cfg(&t, &mut set),
        Err(_) => {
            let _ = std::fs::write(CFG, cfg_text(&set));
        }
    }
    if let Some(v) = arg("--speed") {
        set.speed = v.parse().unwrap_or(set.speed);
    }
    if let Some(v) = arg("--scale") {
        set.scale = v.parse().unwrap_or(set.scale);
    }
    set.borderless |= args.iter().any(|a| a == "--borderless");
    set.topmost |= args.iter().any(|a| a == "--topmost");
    set.speed = set.speed.clamp(8, 5000);

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
        .map_or(0, |d| (d.as_millis() / set.speed as u128) as u32)
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
        render(&w, &mut buf, (0, 0), ZOOM_MIN);
        write_bmp(&path, &buf).expect("write bmp");
        return;
    }

    // ponytail: the world grid is fixed, so the scale setting resizes the view of it.
    // Panning and zooming move the camera over the same 90x66 world.
    let mut win = open_window(&set);

    let mut last = Instant::now();
    let (mut paused, mut menu, mut sel, mut quit, mut dirty) = (false, false, 0usize, false, true);
    let mut zoom = ZOOM_MIN;
    let mut cam = (0i32, 0i32);
    // What a held left button is doing: panning the map, or carrying the whole window.
    let mut grab: Option<((i32, i32), (i32, i32), bool)> = None;
    let mut was_down = false;
    while win.is_open() && !quit {
        // Esc opens settings, it does not quit: this thing is meant to be left running.
        if win.is_key_pressed(Key::Escape, KeyRepeat::No) {
            menu = !menu;
            if !menu {
                let _ = std::fs::write(CFG, cfg_text(&set));
            }
            dirty = true;
        }
        if win.is_key_pressed(Key::Q, KeyRepeat::No) {
            quit = true;
        }
        if menu {
            if win.is_key_pressed(Key::Down, KeyRepeat::Yes) {
                sel = (sel + 1) % Settings::ROWS;
            }
            if win.is_key_pressed(Key::Up, KeyRepeat::Yes) {
                sel = (sel + Settings::ROWS - 1) % Settings::ROWS;
            }
            let d = win.is_key_pressed(Key::Right, KeyRepeat::Yes) as i32
                - win.is_key_pressed(Key::Left, KeyRepeat::Yes) as i32;
            if d != 0 {
                if set.adjust(sel, d) {
                    win = open_window(&set);
                } else {
                    win.topmost(set.topmost);
                }
                dirty = true;
            }
        } else if win.is_key_pressed(Key::Equal, KeyRepeat::No) {
            set.adjust(0, 1);
        } else if win.is_key_pressed(Key::Minus, KeyRepeat::No) {
            set.adjust(0, -1);
        }
        if win.is_key_pressed(Key::Space, KeyRepeat::No) {
            paused = !paused;
        }

        // Mouse. Positions come back in buffer pixels, which is what everything drawn
        // here is measured in, so no conversion.
        let mouse = win.get_mouse_pos(minifb::MouseMode::Discard);
        let down = win.get_mouse_down(minifb::MouseButton::Left);
        let click = down && !was_down;
        was_down = down;
        let on_map = mouse.is_some_and(|(mx, my)| {
            (MAP_X..MAP_X + MAP_PW).contains(&(mx as usize))
                && (MAP_Y..MAP_Y + MAP_PH).contains(&(my as usize))
        });

        if let Some((mx, my)) = mouse {
            let (mx, my) = (mx as usize, my as usize);
            if menu && click {
                match menu_hit(mx, my) {
                    Some((row, d)) if row != usize::MAX => {
                        sel = row;
                        if d != 0 {
                            if set.adjust(row, d) {
                                win = open_window(&set);
                            } else {
                                win.topmost(set.topmost);
                            }
                        }
                    }
                    Some(_) => {}
                    None => {
                        menu = false;
                        let _ = std::fs::write(CFG, cfg_text(&set));
                    }
                }
                dirty = true;
            } else if !menu && click && my >= WIN_H - 32 && mx >= PANEL_X {
                menu = true; // the hint line doubles as the settings button
                dirty = true;
            }
        }

        // Wheel zoom, anchored on the cursor so the cell under it stays put.
        if let Some((_, sy)) = win.get_scroll_wheel() {
            if sy != 0.0 && !menu {
                let old = zoom;
                zoom = if sy > 0.0 {
                    (zoom + 4).min(ZOOM_MAX)
                } else {
                    zoom.saturating_sub(4).max(ZOOM_MIN)
                };
                if zoom != old {
                    let (ax, ay) = mouse
                        .filter(|_| on_map)
                        .map_or((MAP_PW as f32 / 2.0, MAP_PH as f32 / 2.0), |(mx, my)| {
                            (mx - MAP_X as f32, my - MAP_Y as f32)
                        });
                    let k = zoom as f32 / old as f32;
                    cam.0 = ((cam.0 as f32 + ax) * k - ax) as i32;
                    cam.1 = ((cam.1 as f32 + ay) * k - ay) as i32;
                    cam = clamp_cam(cam, zoom);
                    dirty = true;
                }
            }
        }

        // Dragging: on the map it pans, on the surrounding chrome it carries a frameless
        // window, which otherwise has no title bar to grab.
        if !menu {
            let hwnd = win.get_window_handle();
            if click {
                let (cur, wpos) = cursor_and_window(hwnd);
                grab = Some((cur, if on_map { cam } else { wpos }, on_map));
            } else if !down {
                grab = None;
            }
            if let Some((anchor, start, panning)) = grab {
                if down {
                    let (cur, _) = cursor_and_window(hwnd);
                    let (dx, dy) = (cur.0 - anchor.0, cur.1 - anchor.1);
                    if panning {
                        let next = clamp_cam((start.0 - dx, start.1 - dy), zoom);
                        if next != cam {
                            cam = next;
                            dirty = true;
                        }
                    } else if (dx, dy) != (0, 0) && set.borderless {
                        win.set_position((start.0 + dx) as isize, (start.1 + dy) as isize);
                    }
                }
            }
        }
        // Arrows pan when the settings screen is not using them.
        if !menu {
            let step = (zoom * 2) as i32;
            let dx = win.is_key_down(Key::Right) as i32 - win.is_key_down(Key::Left) as i32;
            let dy = win.is_key_down(Key::Down) as i32 - win.is_key_down(Key::Up) as i32;
            if (dx, dy) != (0, 0) {
                let next = clamp_cam((cam.0 + dx * step, cam.1 + dy * step), zoom);
                if next != cam {
                    cam = next;
                    dirty = true;
                }
            }
        }

        if !paused && !menu && last.elapsed() >= Duration::from_millis(set.speed) {
            last = Instant::now();
            w.tick();
            dirty = true;
            if w.year % 25 == 0 {
                let _ = std::fs::write(SAVE, format!("{seed} {}", w.year));
            }
        }
        // The menu redraws the world under itself every frame, otherwise the old
        // selection marker stays behind while the world sits paused.
        if dirty || menu {
            render(&w, &mut buf, cam, zoom);
            if menu {
                draw_menu(&mut buf, sel, &set);
            }
            dirty = false;
            let _ = win.update_with_buffer(&buf, WIN_W, WIN_H);
        } else {
            // Pump input without pushing 2.4 MB the window already has. Blitting every
            // loop regardless of change was costing about 4% of a core to show a
            // picture that changes five times a second.
            win.update();
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    let _ = std::fs::write(SAVE, format!("{seed} {}", w.year));
    let _ = std::fs::write(CFG, cfg_text(&set));
}

#[test]
#[ignore]
fn bench() {
    let mut w = World::new(7);
    for _ in 0..2000 {
        w.tick();
    }
    let t = Instant::now();
    for _ in 0..20000 {
        w.tick();
    }
    let tick = t.elapsed().as_secs_f64() / 20000.0 * 1e6;

    let mut buf = vec![0u32; WIN_W * WIN_H];
    let t = Instant::now();
    for _ in 0..2000 {
        render(&w, &mut buf, (0, 0), ZOOM_MIN);
    }
    let out = t.elapsed().as_secs_f64() / 2000.0 * 1e6;
    let t = Instant::now();
    for _ in 0..2000 {
        render(&w, &mut buf, (300, 200), 16);
    }
    let zoomed = t.elapsed().as_secs_f64() / 2000.0 * 1e6;
    println!(
        "tick {tick:.1} us   render {out:.1} us   render zoomed {zoomed:.1} us   60fps budget 16666 us"
    );
}

/// The settings screen can't be clicked from a test, so the keystroke logic and the
/// file round trip are checked directly instead.
#[test]
fn settings_keys_and_file_round_trip() {
    let mut s = Settings {
        speed: 200,
        scale: 1,
        borderless: false,
        topmost: false,
    };

    assert!(!s.adjust(0, 1), "speed needs no new window");
    assert_eq!(s.speed, 100, "right is faster");
    for _ in 0..20 {
        s.adjust(0, 1);
    }
    assert_eq!(s.speed, 8, "speed clamps instead of reaching zero and dividing by it");
    for _ in 0..20 {
        s.adjust(0, -1);
    }
    assert_eq!(s.speed, 5000);

    // 3 is not a minifb scale, so the row must step over it in both directions.
    for want in [2, 4, 4] {
        assert!(s.adjust(1, 1));
        assert_eq!(s.scale, want);
    }
    for want in [2, 1, 1] {
        s.adjust(1, -1);
        assert_eq!(s.scale, want);
    }

    assert!(s.adjust(2, 1), "frame change needs the window reopened");
    assert!(s.borderless);
    assert!(!s.adjust(3, 1), "topmost applies live");
    assert!(s.topmost);

    // Whatever the menu writes has to read back identically, or closing the settings
    // screen would silently revert them on the next launch.
    let mut back = Settings {
        speed: 1,
        scale: 1,
        borderless: false,
        topmost: false,
    };
    parse_cfg(&cfg_text(&s), &mut back);
    assert_eq!(s, back);
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
