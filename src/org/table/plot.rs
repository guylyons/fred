//! org-plot.el: plot a table with gnuplot. The `#+PLOT:` options, the
//! data file (TSV, or grid data), the gnuplot script for each plot type
//! (2d, 3d, grid, radar) are built as upstream does; the script is then
//! run with the external `gnuplot` program (an error says so when it is
//! not installed). Emacs' gnuplot-mode process buffer has no
//! counterpart: gnuplot runs detached, `-persist` keeping its window.

use super::doc::{Doc, NUMBER_RE, Row, lazy_re, re};
use super::lisp::{self, L};
use super::orgtbl::{Params, to_generic};
use std::collections::BTreeMap;

/// The plot parameters (a plist from keyword to Lisp value).
#[derive(Clone, Debug, Default)]
pub struct Plot(pub Vec<(String, L)>);

impl Plot {
    pub fn get(&self, k: &str) -> Option<&L> {
        self.0.iter().find(|(x, _)| x == k).map(|(_, v)| v).filter(|v| v.truthy())
    }

    pub fn member(&self, k: &str) -> bool {
        self.0.iter().any(|(x, _)| x == k)
    }

    pub fn put(&mut self, k: &str, v: L) {
        match self.0.iter_mut().find(|(x, _)| x == k) {
            Some(slot) => slot.1 = v,
            None => self.0.push((k.into(), v)),
        }
    }

    fn str(&self, k: &str) -> Option<String> {
        self.get(k).map(|v| match v {
            L::Str(s) => s.to_string(),
            o => lisp::princ(o),
        })
    }

    fn int(&self, k: &str) -> Option<i64> {
        match self.get(k)? {
            L::Int(i) => Some(*i),
            L::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    fn sym(&self, k: &str) -> Option<String> {
        self.get(k).map(lisp::princ)
    }
}

const OPTIONS: &[(&str, &str)] = &[
    ("type", ":plot-type"),
    ("script", ":script"),
    ("line", ":line"),
    ("set", ":set"),
    ("title", ":title"),
    ("ind", ":ind"),
    ("deps", ":deps"),
    ("with", ":with"),
    ("file", ":file"),
    ("labels", ":labels"),
    ("map", ":map"),
    ("timeind", ":timeind"),
    ("timefmt", ":timefmt"),
    ("min", ":ymin"),
    ("ymin", ":ymin"),
    ("max", ":ymax"),
    ("ymax", ":ymax"),
    ("xmin", ":xmin"),
    ("xmax", ":xmax"),
    ("ticks", ":ticks"),
    ("trans", ":transpose"),
    ("transpose", ":transpose"),
];

/// org-plot/add-options-to-plist.
pub fn add_options(p: &mut Plot, options: &str) {
    for (name, key) in OPTIONS {
        let r = re(&format!(r#"{}:("[^"]+?"|\([^)]+?\)|[^ \t\n\r;,.]*)"#, regex::escape(name)));
        if matches!(*name, "set" | "line") {
            for m in r.captures_iter(options) {
                let v = lisp::read(&m[1]).unwrap_or(L::Nil);
                let mut list = p.get(key).cloned().map_or(vec![], |x| x.items().unwrap_or_default());
                list.insert(0, v);
                p.put(key, lisp::list(list));
            }
        } else if let Some(m) = r.captures(options) {
            p.put(key, lisp::read(&m[1]).unwrap_or(L::Nil));
        }
    }
}

lazy_re!(PLOT_LINE, r"(?i)#\+PLOT: +(.*)$");
lazy_re!(TS3, r"[\[<]([0-9]{4})-([0-9]{2})-([0-9]{2})(?: [^\]>\s0-9+-]+)?(?: ([0-9]{1,2}):([0-9]{2}))?[^\]>\n]*[\]>]");

/// org-plot/goto-nearest-table: the first table line at or after point.
pub fn goto_nearest_table(d: &mut Doc) -> Result<(), String> {
    d.bol();
    while !d.at_table() {
        if d.l + 1 >= d.n() {
            return Err("No table found".into());
        }
        d.goto(d.l + 1, 0);
    }
    let b = d.table_begin();
    d.goto(b, 0);
    Ok(())
}

/// A timestamp in org-plot-timestamp-fmt.
fn format_timestamp(s: &str, fmt: &str) -> String {
    let Some(m) = TS3.captures(s) else {
        return s.to_owned();
    };
    let v: Vec<u32> = (1..=5).map(|i| m.get(i).map_or(0, |x| x.as_str().parse().unwrap_or(0))).collect();
    let mut out = String::new();
    let mut it = fmt.chars();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('Y') => out.push_str(&format!("{:04}", v[0])),
            Some('m') => out.push_str(&format!("{:02}", v[1])),
            Some('d') => out.push_str(&format!("{:02}", v[2])),
            Some('H') => out.push_str(&format!("{:02}", v[3])),
            Some('M') => out.push_str(&format!("{:02}", v[4])),
            Some('S') => out.push_str("00"),
            Some('s') => {
                let days = super::days_from_civil(v[0] as i64, v[1], v[2]);
                out.push_str(&(days * 86400 + v[3] as i64 * 3600 + v[4] as i64 * 60).to_string());
            }
            Some(o) => {
                out.push('%');
                out.push(o);
            }
            None => out.push('%'),
        }
    }
    out
}

/// org-plot-quote-tsv-field.
pub fn quote_tsv_field(s: &str, timefmt: &str) -> String {
    if re(NUMBER_RE).is_match(s) {
        s.to_owned()
    } else if TS3.is_match(s) {
        format_timestamp(s, timefmt)
    } else {
        format!("\"{}\"", s.split('"').collect::<Vec<_>>().join("\"\""))
    }
}

/// org-plot/gnuplot-to-data: the TSV text.
pub fn to_data(table: &[Row], params: &Plot) -> Result<String, String> {
    let timefmt = params.str(":timefmt").unwrap_or_else(|| "%Y-%m-%d-%H:%M:%S".into());
    let rows: Vec<Row> = table
        .iter()
        .map(|r| match r {
            Row::Cells(c) => Row::Cells(c.iter().map(|x| if x.is_empty() { String::new() } else { quote_tsv_field(x, &timefmt) }).collect()),
            Row::Hline => Row::Hline,
        })
        .collect();
    let p = Params::read("(:sep \"\\t\" :with-special-rows nil)")?;
    to_generic(&rows, &p)
}

/// org-plot/gnuplot-to-grid-data: the grid data and the y labels.
pub fn to_grid_data(table: &[Vec<String>], params: &Plot) -> (String, Vec<(usize, String)>) {
    let last_col = table.first().map_or(0, |r| r.len()) as i64 - 1;
    let ind = params.int(":ind").unwrap_or(0) - 1;
    let deps: Vec<i64> = match params.get(":deps") {
        Some(d) if params.member(":deps") => d.items().unwrap_or_default().iter().filter_map(|x| if let L::Int(i) = x { Some(i - 1) } else { None }).collect(),
        _ => (0..=last_col).collect(),
    };
    let deps: Vec<i64> = deps.into_iter().filter(|&c| c != ind).collect();
    let row_vals: Vec<(usize, String)> = if ind < 0 {
        vec![]
    } else {
        table.iter().enumerate().map(|(i, r)| (i + 1, r.get(ind as usize).cloned().unwrap_or_default())).collect()
    };
    let t: Vec<Vec<String>> = table.iter().map(|r| r.iter().enumerate().filter(|(i, _)| deps.contains(&(*i as i64))).map(|(_, x)| x.clone()).collect()).collect();
    let mut out = String::new();
    let row = |c: f64, r: f64, v: f64| format!("{c:.6}  {:.6}  {v:.6}\n{c:.6}  {:.6}  {v:.6}\n", r + 0.5, r + 1.5);
    let ncol = t.first().map_or(0, Vec::len);
    for c in 0..ncol {
        let mut back = String::new();
        let mut front = String::new();
        for (r, rowv) in t.iter().enumerate() {
            let v = lisp::string_to_number(rowv.get(c).map_or("", String::as_str)).f();
            back.push_str(&row(c as f64, r as f64, v));
            front.push_str(&row(c as f64 + 1.0, r as f64, v));
        }
        out.push_str(&format!("{back}\n{front}\n"));
    }
    (out, row_vals)
}

/// org--plot/values-stats.
fn values_stats(nums: &[f64], hard_min: Option<f64>, hard_max: Option<f64>) -> BTreeMap<&'static str, f64> {
    let minimum = hard_min.unwrap_or_else(|| nums.iter().copied().fold(f64::INFINITY, f64::min));
    let maximum = hard_max.unwrap_or_else(|| nums.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    let range = maximum - minimum;
    let order = if range == 0.0 { 0.0 } else { (1.0 - range.log10()).ceil() };
    let factor = 10f64.powf(order);
    let nice_min = if range == 0.0 { nums.first().copied().unwrap_or(0.0) } else { (minimum * factor).floor() / factor };
    let nice_max = if range == 0.0 { nums.first().copied().unwrap_or(0.0) } else { (maximum * factor).ceil() / factor };
    BTreeMap::from([
        ("min", minimum),
        ("max", maximum),
        ("range", range),
        ("range-factor", factor),
        ("nice-min", nice_min),
        ("nice-max", nice_max),
        ("nice-range", nice_max - nice_min),
    ])
}

/// org--plot/prime-factors.
fn prime_factors(mut value: i64) -> Vec<i64> {
    let mut factors = vec![];
    let mut i = 2;
    while value > 1 && i <= value {
        if value % i == 0 {
            factors.insert(0, i);
            value /= i;
        } else {
            i += 1;
        }
    }
    factors
}

/// org--plot/sensible-tick-num.
fn sensible_tick_num(table: &[Vec<String>], hard_min: Option<f64>, hard_max: Option<f64>) -> i64 {
    let mut weights: Vec<(i64, f64)> = vec![];
    for row in table {
        let nums: Vec<f64> = row.iter().skip(1).map(|x| lisp::string_to_number(x).f()).collect();
        if nums.is_empty() {
            continue;
        }
        let s = values_stats(&nums, hard_min, hard_max);
        let mut val = (s["range-factor"] * s["nice-range"]).round() as i64;
        if val % 10 == 0 && val != 0 {
            val /= 10;
        }
        let f = prime_factors(val);
        let n = f.len().max(1) as f64;
        let mut freq: Vec<(i64, f64)> = vec![];
        for x in &f {
            match freq.iter_mut().find(|(k, _)| k == x) {
                Some(slot) => slot.1 += 1.0 / n,
                None => freq.push((*x, 1.0 / n)),
            }
        }
        for (k, v) in freq {
            match weights.iter_mut().find(|(x, _)| *x == k) {
                Some(slot) => slot.1 += v,
                None => weights.push((k, v)),
            }
        }
    }
    weights.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    nice_frequency_pick(&weights).iter().product::<i64>().max(1)
}

/// org--plot/nice-frequency-pick.
fn nice_frequency_pick(f: &[(i64, f64)]) -> Vec<i64> {
    match f.len() {
        0 => vec![1],
        1 => vec![f[0].0],
        2 => {
            if 3.0 <= (f[0].1 / f[1].1).floor() {
                vec![f[0].0, f[0].0]
            } else {
                vec![f[0].0, f[1].0]
            }
        }
        _ => {
            let total: f64 = f.iter().map(|x| x.1).sum();
            let n: Vec<(i64, f64)> = f.iter().map(|(k, v)| (*k, v / total)).collect();
            let mut pick = vec![n[0].0];
            let r12 = n[0].1 / n[1].1;
            let r23 = n[1].1 / n[2].1;
            let r13 = r12 * r23;
            if r12 > 4.0 {
                pick.push(n[0].0);
            }
            if r12 < n[1].0 as f64 && pick.iter().product::<i64>() * n[1].0 < 30 {
                pick.push(n[1].0);
            }
            if r13 < n[2].0 as f64 && pick.iter().product::<i64>() * n[2].0 < 30 {
                pick.push(n[2].0);
            }
            pick
        }
    }
}

const RADAR_TEMPLATE: &str = "### spider plot/chart with gnuplot
# also known as: radar chart, web chart, star chart, cobweb chart,
#                radar plot,  web plot,  star plot,  cobweb plot,  etc. ...
set datafile separator ' '
set size square
unset tics
set angles degree
set key bmargin center horizontal
unset border

# Load data and setup
load \"%s\"

# General settings
DataColCount = words($Data[1])-1
AxesCount = |$Data|-HeaderLines-1
AngleOffset = 90
Max = 1
d=0.1*Max
Direction = -1   # counterclockwise=1, clockwise = -1

# Tic settings
TicCount = %s
TicOffset = 0.1
TicValue(axis,i) = real(i)*(word($Settings[axis],3)-word($Settings[axis],2)) \\
	  / word($Settings[axis],4)+word($Settings[axis],2)
TicLabelPosX(axis,i) = PosX(axis,i/TicCount) + PosY(axis, TicOffset)
TicLabelPosY(axis,i) = PosY(axis,i/TicCount) - PosX(axis, TicOffset)
TicLen = 0.03
TicdX(axis,i) = 0.5*TicLen*cos(alpha(axis)-90)
TicdY(axis,i) = 0.5*TicLen*sin(alpha(axis)-90)

# Label
LabOffset = 0.10
LabX(axis) = PosX(axis+1,Max+2*d) + PosY(axis, LabOffset)
LabY(axis) = PosY($0+1,Max+2*d)

# Functions
alpha(axis) = (axis-1)*Direction*360.0/AxesCount+AngleOffset
PosX(axis,R) = R*cos(alpha(axis))
PosY(axis,R) = R*sin(alpha(axis))
Scale(axis,value) = real(value-word($Settings[axis],2))/(word($Settings[axis],3)-word($Settings[axis],2))

# Spider settings
set style arrow 1 dt 1 lw 1.0 @fgal head filled size 0.06,25     # style for axes
set style arrow 2 dt 2 lw 0.5 @fgal nohead   # style for weblines
set style arrow 3 dt 1 lw 1 @fgal nohead     # style for axis tics
set samples AxesCount
set isosamples TicCount
set urange[1:AxesCount]
set vrange[1:TicCount]
set style fill transparent solid 0.2

set xrange[-Max-4*d:Max+4*d]
set yrange[-Max-4*d:Max+4*d]
plot \\
    '+' u (0):(0):(PosX($0,Max+d)):(PosY($0,Max+d)) w vec as 1 not, \\
    $Data u (LabX($0)): \\
	(LabY($0)):1 every ::HeaderLines w labels center enhanced @fgt not, \\
    for [i=1:DataColCount] $Data u (PosX($0+1,Scale($0+1,column(i+1)))): \\
	(PosY($0+1,Scale($0+1,column(i+1)))) every ::HeaderLines w filledcurves lt i title word($Data[1],i+1), \\
%s
#    '++' u (PosX($1,$2/TicCount)-TicdX($1,$2/TicCount)): \\
#        (PosY($1,$2/TicCount)-TicdY($1,$2/TicCount)): \\
#        (2*TicdX($1,$2/TicCount)):(2*TicdY($1,$2/TicCount)) \\
#        w vec as 3 not, \\
### end of code
";

const RADAR_TICKS: &str = "    '++' u (PosX($1,$2/TicCount)):(PosY($1,$2/TicCount)): \\
	(PosX($1+1,$2/TicCount)-PosX($1,$2/TicCount)):  \\
	(PosY($1+1,$2/TicCount)-PosY($1,$2/TicCount)) w vec as 2 not, \\
    '++' u (TicLabelPosX(%s,$2)):(TicLabelPosY(%s,$2)): \\
	(sprintf('%%g',TicValue(%s,$2))) w labels font ',8' @fgat not";

fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut it = args.iter();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('s') => {
                    chars.next();
                    out.push_str(it.next().copied().unwrap_or(""));
                    continue;
                }
                Some('%') => {
                    chars.next();
                    out.push('%');
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

fn num(x: f64) -> String {
    lisp::float_to_string(x)
}

/// org--plot/radar: the script, writing its setup to `setup_file`.
pub fn radar(table: &[Vec<String>], params: &Plot, setup_file: &std::path::Path) -> Result<String, String> {
    let labels: Vec<String> = params.get(":labels").map(|l| l.items().unwrap_or_default().iter().map(lisp::princ).collect()).unwrap_or_default();
    let mut rows: Vec<Vec<String>> = table.to_vec();
    if let Some(first) = table.first() {
        rows.push(first.clone());
    }
    let data = format!(
        "\"{}\"\n{}",
        labels.join("\" \""),
        rows.iter().map(|r| format!("\"{}\" {}", r.first().map_or("", String::as_str), r[1..].join(" "))).collect::<Vec<_>>().join("\n")
    );
    let hmin = params.get(":ymin").map(|v| match v {
        L::Int(i) => *i as f64,
        L::Float(f) => *f,
        _ => 0.0,
    });
    let hmax = params.get(":ymax").map(|v| match v {
        L::Int(i) => *i as f64,
        L::Float(f) => *f,
        _ => 0.0,
    });
    let ticks = params.int(":ticks").unwrap_or_else(|| sensible_tick_num(table, hmin, hmax));
    let settings = rows
        .iter()
        .map(|r| {
            let nums: Vec<f64> = r[1..].iter().map(|x| lisp::string_to_number(x).f()).collect();
            let s = values_stats(&nums, None, None);
            format!(
                "\"{}\" {} {} {}",
                r.first().map_or("", String::as_str),
                hmin.map_or_else(|| num(s["nice-min"]), num),
                hmax.map_or_else(|| num(s["nice-max"]), num),
                if ticks == 0 { 2 } else { ticks }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let setup = format!("# Data\n$Data <<HEREHAVESOMEDATA\n{data}\nHEREHAVESOMEDATA\nHeaderLines = 1\n\n# Settings for scale and offset adjustments\n# axis min max tics axisLabelXoff axisLabelYoff\n$Settings <<EOD\n{settings}\nEOD\n");
    std::fs::write(setup_file, setup).map_err(|e| e.to_string())?;
    let tick_arg = if hmin.is_some() && hmax.is_some() { "1" } else { "$1" };
    let tickstr = if ticks == 0 { String::new() } else { fill(RADAR_TICKS, &[tick_arg, tick_arg, tick_arg]) };
    let setup_s = setup_file.display().to_string();
    let tcount = if ticks == 0 { 2 } else { ticks }.to_string();
    Ok(fill(RADAR_TEMPLATE, &[&setup_s, &tcount, &tickstr]))
}

/// org-plot/gnuplot-script.
pub fn script(table: &[Vec<String>], data_file: &str, num_cols: usize, params: &Plot, preface: bool) -> Result<String, String> {
    let ty = params.sym(":plot-type").unwrap_or_else(|| "2d".into());
    if !matches!(ty.as_str(), "2d" | "3d" | "grid" | "radar") {
        return Err(format!("Org-plot type `{ty}' is undefined"));
    }
    let mut s = String::from("reset");
    let ats = |s: &mut String, line: &str| {
        s.push('\n');
        s.push_str(line);
    };
    let file = params.str(":file");
    let ext = file.as_ref().and_then(|f| std::path::Path::new(f).extension().and_then(|e| e.to_str()).map(str::to_owned));
    let term = super::opt_string("org-plot/gnuplot-term-extra", "");
    ats(&mut s, &format!("set term {} {term}", if file.is_some() { ext.unwrap_or_default() } else { "GNUTERM".into() }));
    if let Some(f) = &file {
        let abs = std::path::absolute(f).map_or_else(|_| f.clone(), |p| p.display().to_string());
        ats(&mut s, &format!("set output '{abs}'"));
    }
    match ty.as_str() {
        "3d" if params.get(":map").is_some() => ats(&mut s, "set map"),
        "grid" => ats(&mut s, if params.get(":map").is_some() { "set pm3d map" } else { "set map" }),
        _ => {}
    }
    ats(&mut s, &super::opt_string("org-plot/gnuplot-script-preamble", ""));
    if let Some(t) = params.str(":title") {
        ats(&mut s, &format!("set title '{t}'"));
    }
    for l in params.get(":line").map(|x| x.items().unwrap_or_default()).unwrap_or_default() {
        ats(&mut s, &lisp::princ(&l));
    }
    for st in params.get(":set").map(|x| x.items().unwrap_or_default()).unwrap_or_default() {
        ats(&mut s, &format!("set {}", lisp::princ(&st)));
    }
    if !s.lines().any(|l| l.starts_with("set datafile separator")) {
        ats(&mut s, "set datafile separator \"\\t\"");
    }
    let labels_of = |k: &str| -> Option<Vec<(String, String)>> {
        let v = params.get(k)?;
        Some(
            v.items()
                .unwrap_or_default()
                .iter()
                .map(|p| match p {
                    L::Dotted(a, b) => (lisp::princ(&a[0]), lisp::princ(b)),
                    L::List(a) => (lisp::princ(&a[0]), a.get(1).map(lisp::princ).unwrap_or_default()),
                    o => (String::new(), lisp::princ(o)),
                })
                .collect(),
        )
    };
    if let Some(x) = labels_of(":xlabels") {
        ats(&mut s, &format!("set xtics ({})", x.iter().map(|(i, l)| format!("\"{l}\" {i}")).collect::<Vec<_>>().join(", ")));
    }
    if let Some(y) = labels_of(":ylabels") {
        ats(&mut s, &format!("set ytics ({})", y.iter().map(|(i, l)| format!("\"{l}\" {i}")).collect::<Vec<_>>().join(", ")));
    }
    if params.get(":timeind").is_some() {
        ats(&mut s, "set xdata time");
        ats(&mut s, &format!("set timefmt \"{}\"", params.str(":timefmt").unwrap_or_else(|| "%Y-%m-%d-%H:%M:%S".into())));
    }
    if !preface {
        let with = if ty == "grid" { "pm3d".to_owned() } else { params.sym(":with").unwrap_or_else(|| "lines".into()) };
        let (cmd, lines): (Option<&str>, Vec<String>) = match ty.as_str() {
            "2d" => {
                let ind = params.int(":ind");
                let deps: Option<Vec<i64>> = params.get(":deps").map(|d| d.items().unwrap_or_default().iter().filter_map(|x| if let L::Int(i) = x { Some(*i) } else { None }).collect());
                let text_ind = params.get(":textind").is_some() || params.sym(":with").as_deref() == Some("histograms");
                let labels: Vec<String> = params.get(":labels").map(|l| l.items().unwrap_or_default().iter().map(lisp::princ).collect()).unwrap_or_default();
                let mut out = vec![];
                for col in 0..num_cols {
                    let c = col as i64 + 1;
                    if ind == Some(c) || deps.as_ref().is_some_and(|d| !d.contains(&c)) {
                        continue;
                    }
                    let using = match ind {
                        Some(i) if i > 0 && !text_ind => format!("{i}:"),
                        _ => String::new(),
                    };
                    let xtic = if text_ind { format!(":xticlabel({})", ind.unwrap_or(0)) } else { String::new() };
                    let title = labels.get(col).cloned().unwrap_or_else(|| c.to_string());
                    out.push(format!("'{data_file}' using {using}{c}{xtic} with {with} title '{title}'"));
                }
                (Some("plot"), out)
            }
            "3d" => (Some("splot"), vec![format!("'{data_file}' matrix with {with} title ''")]),
            "grid" => (Some("splot"), vec![format!("'{data_file}' with {with} title ''")]),
            _ => {
                let setup = std::env::temp_dir().join(format!("org-plot-setup-{}", std::process::id()));
                (None, vec![radar(table, params, &setup)?])
            }
        };
        let body = lines.join(",\\\n    ");
        ats(&mut s, &match cmd {
            Some(c) => format!("{c} {body}"),
            None => body,
        });
    }
    Ok(s)
}

/// Collect the plot parameters for the table at point and its data.
pub fn prepare(d: &mut Doc, given: Plot) -> Result<(Plot, Vec<Vec<String>>, usize), String> {
    let mut params = given;
    for (k, v) in [(":plot-type", "2d"), (":with", "lines"), (":ind", "0")] {
        if !params.member(k) {
            params.put(k, lisp::read(v).unwrap());
        }
    }
    let (l0, c0) = (d.l, d.c);
    goto_nearest_table(d)?;
    let tb = d.l;
    let mut l = tb;
    let mut opts = vec![];
    while l > 0 && d.lines[l - 1].trim_start().starts_with("#+") {
        l -= 1;
        if let Some(m) = PLOT_LINE.captures(&d.lines[l]) {
            opts.push(m[1].to_owned());
        }
    }
    for o in &opts {
        add_options(&mut params, o);
    }
    let mut table = d.to_lisp();
    d.goto(l0, c0);
    if matches!(params.sym(":transpose").as_deref(), Some("y" | "yes" | "t")) {
        let has_hline = table.contains(&Row::Hline);
        let rows: Vec<&Vec<String>> = table.iter().filter_map(Row::cells).collect();
        let n = rows.iter().map(|r| r.len()).min().unwrap_or(0);
        let mut t: Vec<Row> = (0..n).map(|j| Row::Cells(rows.iter().map(|r| r[j].clone()).collect())).collect();
        if has_hline && !t.is_empty() {
            t.insert(1, Row::Hline);
        }
        table = t;
    }
    let num_cols = match table.first() {
        Some(Row::Hline) => table.get(1).and_then(Row::cells).map_or(0, Vec::len),
        Some(Row::Cells(c)) => c.len(),
        None => 0,
    };
    if table.get(1) == Some(&Row::Hline) {
        if !params.member(":labels")
            && let Some(Row::Cells(h)) = table.first()
        {
            params.put(":labels", lisp::list(h.iter().map(|x| lisp::str_(x)).collect()));
        }
        table = table[1..].iter().filter(|r| **r != Row::Hline).cloned().collect();
    }
    let rows: Vec<Vec<String>> = table.iter().filter_map(Row::cells).cloned().collect();
    // Check the type of the ind column.
    let ty = params.sym(":plot-type").unwrap_or_default();
    if ty == "2d" {
        let ind = params.int(":ind").unwrap_or(0) - 1;
        if ind >= 0 {
            let col: Vec<&str> = rows.iter().map(|r| r.get(ind as usize).map_or("", String::as_str)).collect();
            let num = re(NUMBER_RE);
            if col.iter().all(|x| TS3.is_match(x)) {
                params.put(":timeind", L::T);
            } else if params.sym(":with").as_deref() == Some("hist") || !col.iter().all(|x| num.is_match(x)) {
                params.put(":textind", L::T);
            }
        }
    }
    Ok((params, rows, num_cols))
}

/// org-plot/gnuplot: write the data and script and run gnuplot.
pub fn gnuplot(d: &mut Doc, given: Plot) -> Result<(), String> {
    let (params, rows, num_cols) = prepare(d, given)?;
    let ty = params.sym(":plot-type").unwrap_or_default();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(format!("{rows:?}{num_cols}{:?}", params.0)), &mut h);
    let data_file = std::env::temp_dir().join(format!("org-plot-{:x}", std::hash::Hasher::finish(&h)));
    let mut params = params;
    let data = if ty == "grid" {
        let (text, ylabels) = to_grid_data(&rows, &params);
        if !ylabels.is_empty() {
            params.put(":ylabels", lisp::list(ylabels.into_iter().map(|(i, l)| L::Dotted(std::rc::Rc::new(vec![L::Int(i as i64)]), std::rc::Rc::new(lisp::str_(&l)))).collect()));
        }
        text
    } else {
        to_data(&rows.iter().map(|r| Row::Cells(r.clone())).collect::<Vec<_>>(), &params)?
    };
    let df = data_file.display().to_string();
    let user_script = params.str(":script");
    let mut script_text = script(&rows, &df, num_cols, &params, user_script.is_some())?;
    if let Some(us) = &user_script {
        let content = std::fs::read_to_string(super::super::options::expand(us)).map_err(|e| format!("{us}: {e}"))?;
        script_text = format!("{script_text}\n{}", content.replace("$datafile", &df));
    }
    let Some(gp) = super::lisp::which("gnuplot") else {
        return Err("org-plot/gnuplot: the gnuplot program is not installed (it must be on PATH)".into());
    };
    std::fs::write(&data_file, data).map_err(|e| e.to_string())?;
    let script_file = data_file.with_extension("gp");
    std::fs::write(&script_file, &script_text).map_err(|e| e.to_string())?;
    std::process::Command::new(gp)
        .arg("-persist")
        .arg(&script_file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("gnuplot: {e}"))?;
    d.message(match params.str(":file") {
        Some(f) => format!("Plot written to {f}"),
        None => "Plot sent to gnuplot".into(),
    });
    Ok(())
}
