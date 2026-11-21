//! `wirebug fmt`: `.wb` source laid out in the house style.
//!
//! The formatter reads the lexer's tokens with their trivia attached
//! ([`AttachedTokens`]), cuts them into statements, and prints every token
//! again, as written, in a fresh layout: four spaces a block deep, one
//! statement a line, single spaces between tokens, and the statements of a
//! run of one kind aligned in columns (ports, instances and other
//! `name: value` lines, `use` lines, wires and their endpoints, includes).
//! Comments stay with the statement they lead or end, and blank lines
//! between statements stay, collapsed to one. The tokens are never
//! changed, so the file means what it meant; only a comma closing a list
//! (`[a, b,]`) is left out, which the parser reads the same without.
//!
//! Only a source that lexes and parses without a single error is
//! formatted: in a broken one, the statements are not what the author
//! meant, and laying them out would move the mistake.

mod print;
mod tree;

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::dsl::diagnostics::Problem;
use crate::dsl::lex::trivia::AttachedTokens;
use crate::dsl::lex::{LexError, lex, significant};
use crate::dsl::parse::{Completeness, ParseError, parse_file};
use crate::dsl::project;
use crate::dsl::span::FileId;
use print::Printer;

/// A `.wb` source in the house layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formatted(String);

/// Why a source was not formatted.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Unformattable {
    /// The source does not lex.
    #[error("the source does not lex")]
    Lex(LexError),
    /// The source does not parse: its errors, in source order.
    #[error("the source has {} syntax error(s)", .0.len())]
    Parse(Vec<ParseError>),
}

impl Formatted {
    /// Lay out `src`, refusing a source with a lex or parse error.
    pub fn new(src: &str) -> Result<Self, Unformattable> {
        let lexemes = lex(src, FileId(0)).map_err(Unformattable::Lex)?;
        let parsed = parse_file(significant(&lexemes), FileId(0), src);
        if !parsed.is_ok() || parsed.completeness != Completeness::Whole {
            return Err(Unformattable::Parse(parsed.errors));
        }
        Ok(Formatted(Printer::print(&AttachedTokens::collect(
            src, &lexemes,
        ))))
    }

    /// The formatted text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether formatting changes `src`, the source this was made from.
    pub fn changes(&self, src: &str) -> bool {
        self.0 != src
    }
}

impl From<Formatted> for String {
    fn from(formatted: Formatted) -> Self {
        formatted.0
    }
}

/// One file of a project, formatted.
#[derive(Debug)]
pub struct FormattedFile {
    /// The file, as the loader found it (canonical).
    pub path: PathBuf,
    /// The file's text in the house layout.
    pub formatted: Formatted,
    /// Whether the formatted text differs from the file's.
    pub changed: bool,
}

impl FormattedFile {
    /// Write the formatted text over the file.
    pub fn write(&self) -> std::io::Result<()> {
        std::fs::write(&self.path, self.formatted.as_str())
    }
}

/// Every `.wb` file a project loads, formatted, and the problems that kept
/// files from being formatted.
#[derive(Debug)]
#[must_use]
pub struct ProjectFormat {
    /// The files that parse, in the order the project loads them.
    pub files: Vec<FormattedFile>,
    /// Why the project or some of its files could not be formatted: no
    /// project found, a file that cannot be read, lex and parse errors.
    pub problems: Vec<Problem>,
}

impl ProjectFormat {
    /// Format the project `target` names — a manifest, a project
    /// directory, a `.wb` file (and the files it reaches through `use` and
    /// `fragment`), or, with none, the project around the current
    /// directory — as `check` finds and loads it. Nothing is written.
    pub fn load(target: Option<&Path>) -> Self {
        let entry = match project::discover(target) {
            Ok(entry) => entry,
            Err(problem) => {
                return ProjectFormat {
                    files: Vec::new(),
                    problems: vec![problem],
                };
            }
        };
        let (project, problems) = project::load(&entry);
        let files = project
            .map(|project| {
                project
                    .files
                    .into_iter()
                    .filter_map(|file| {
                        // A file that does not parse has had its errors
                        // reported by the load.
                        let formatted = Formatted::new(&file.src).ok()?;
                        Some(FormattedFile {
                            changed: formatted.changes(&file.src),
                            path: file.path,
                            formatted,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        ProjectFormat {
            files,
            problems: problems
                .into_iter()
                .filter(Problem::stops_formatting)
                .collect(),
        }
    }

    /// The files formatting changes.
    pub fn changed(&self) -> impl Iterator<Item = &FormattedFile> {
        self.files.iter().filter(|file| file.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(src: &str) -> String {
        let formatted = Formatted::new(src).expect("formats").as_str().to_owned();
        let again = Formatted::new(&formatted).expect("formats again");
        assert_eq!(again.as_str(), formatted, "formatting is idempotent");
        formatted
    }

    #[test]
    fn blocks_indent_four_spaces_and_statements_take_a_line_each() {
        assert_eq!(
            fmt("component Lamp{pub port a \"A\";  pub port b \"B\" ;\n\
                 connector x \"X\"{\npub port c \"C\" pin 1;}}"),
            "component Lamp {\n    pub port a \"A\";\n    pub port b \"B\";\n    \
             connector x \"X\" {\n        pub port c \"C\" pin 1;\n    }\n}\n"
        );
    }

    #[test]
    fn tokens_are_single_spaced_with_none_inside_brackets() {
        assert_eq!(
            fmt(
                "view schematic \"V\" {\n  include  l  at ( -2 ,0.5 ) ports {\n west : a , b ; }\n}"
            ),
            "view schematic \"V\" {\n    include l at (-2, 0.5) ports {\n        west: a, b;\n    }\n}\n"
        );
        assert_eq!(
            fmt("component A { wire green / yellow 1.5 [ a . b , c ] {\n length : 150mm ; } }"),
            "component A {\n    wire green/yellow 1.5 [a.b, c] {\n        length: 150mm;\n    }\n}\n"
        );
    }

    #[test]
    fn ports_in_a_run_align_their_labels_and_pins() {
        assert_eq!(
            fmt("component A {\n\
                 pub port batt \"BATT\" pin 1;\n\
                 pub port ign \"IGN\" pins [2, 3];\n\
                 pub port crank \"CRANK\";\n\
                 }\n"),
            "component A {\n\
             \x20   pub port batt  \"BATT\" pin 1;\n\
             \x20   pub port ign   \"IGN\"  pins [2, 3];\n\
             \x20   pub port crank \"CRANK\";\n\
             }\n"
        );
    }

    #[test]
    fn a_blank_line_a_block_or_another_kind_ends_a_run() {
        assert_eq!(
            fmt("component A {\n\
                 pub port a \"A\";\n\
                 pub port long_name \"L\";\n\
                 \n\
                 pub port b \"B\";\n\
                 port private_one \"P\";\n\
                 pub port c \"C\";\n\
                 }\n"),
            "component A {\n\
             \x20   pub port a         \"A\";\n\
             \x20   pub port long_name \"L\";\n\
             \n\
             \x20   pub port b \"B\";\n\
             \x20   port private_one \"P\";\n\
             \x20   pub port c \"C\";\n\
             }\n"
        );
    }

    #[test]
    fn instances_align_their_types_and_labels() {
        assert_eq!(
            fmt("component V {\n\
                 front: FrontBattery \"Front\";\n\
                 charger: ObcDcdc \"OBC\";\n\
                 m: Motor;\n\
                 part: \"X-1\";\n\
                 }\n"),
            "component V {\n\
             \x20   front:   FrontBattery \"Front\";\n\
             \x20   charger: ObcDcdc      \"OBC\";\n\
             \x20   m:       Motor;\n\
             \x20   part:    \"X-1\";\n\
             }\n"
        );
    }

    #[test]
    fn wires_align_gauge_label_and_each_endpoint() {
        assert_eq!(
            fmt("component V {\n\
                 wire red 1.5 \"EV 12V\" [fuses.ev_12v, pump.v12];\n\
                 wire blue 0.5 \"PWM\" [vcm.pwm, pump.pwm, x.y];\n\
                 wire black 6 [a.gnd, b.gnd,];\n\
                 wire yellow/black 0.25 [aa.gnd, b];\n\
                 }\n"),
            "component V {\n\
             \x20   wire red  1.5 \"EV 12V\" [fuses.ev_12v, pump.v12];\n\
             \x20   wire blue 0.5 \"PWM\"    [vcm.pwm,      pump.pwm, x.y];\n\
             \x20   wire black        6    [a.gnd,  b.gnd];\n\
             \x20   wire yellow/black 0.25 [aa.gnd, b];\n\
             }\n"
        );
    }

    #[test]
    fn uses_and_includes_align() {
        assert_eq!(
            fmt(
                "use Lamp from \"lamp.wb\";\nuse BigSwitch from \"switch.wb\";\n\n\
                 view harness \"H\" {\ninclude l.x at (0, 0);\ninclude sw.main at (4, 0);\n}\n"
            ),
            "use Lamp      from \"lamp.wb\";\nuse BigSwitch from \"switch.wb\";\n\n\
             view harness \"H\" {\n    include l.x     at (0, 0);\n    include sw.main at (4, 0);\n}\n"
        );
    }

    #[test]
    fn comments_stay_with_their_statements_and_trailing_ones_align() {
        let src = "// File header.\n\n\n\
                   component A { // the A\n\
                   \x20 // Inputs.\n\
                   pub port batt \"BATT\" pin 1;   // fused feed\n\
                   pub port ignition \"IGN\" pin 2; // run\n\
                   \n\n\
                   // Trailing note.\n\n\
                   } // end of A\n\
                   // EOF note\n\n";
        assert_eq!(
            fmt(src),
            "// File header.\n\
             \n\
             component A { // the A\n\
             \x20   // Inputs.\n\
             \x20   pub port batt     \"BATT\" pin 1; // fused feed\n\
             \x20   pub port ignition \"IGN\"  pin 2; // run\n\
             \n\
             \x20   // Trailing note.\n\
             } // end of A\n\
             // EOF note\n"
        );
    }

    #[test]
    fn a_comment_inside_a_statement_breaks_it_there() {
        let src = "view schematic \"V\" {\ninclude lamp // left\n at (0, 0);\n}\n";
        assert_eq!(
            fmt(src),
            "view schematic \"V\" {\n\
             \x20   include lamp // left\n\
             \x20       at (0, 0);\n\
             }\n"
        );
    }

    #[test]
    fn a_list_split_over_lines_takes_an_item_a_line() {
        let src = "component A {\n\
                   wire black 6.0 [\n\
                   \x20   charger.out_neg, aux.neg, // the bus\n\
                   \x20   // the cooling\n\
                   \x20   pump.gnd, fan.gnd\n\
                   ];\n\
                   wire red 1 [a.b, c.d,];\n\
                   }\n";
        assert_eq!(
            fmt(src),
            "component A {\n\
             \x20   wire black 6.0 [\n\
             \x20       charger.out_neg,\n\
             \x20       aux.neg, // the bus\n\
             \x20       // the cooling\n\
             \x20       pump.gnd,\n\
             \x20       fan.gnd,\n\
             \x20   ];\n\
             \x20   wire red 1 [a.b, c.d];\n\
             }\n"
        );
    }

    #[test]
    fn a_one_statement_block_written_on_one_line_stays_there_and_aligns() {
        let src = "view schematic \"V\" {\n\
                   include bl_left at (26, 3) ports { west: stop; }\n\
                   include chmsl at (4, 3) ports {   west : stop;  }\n\
                   include tail at (8, 3);\n\
                   include x at (1, 1) ports { west: a; east: b; }\n\
                   include y at (2, 2) ports {\nwest: a; }\n\
                   }\n";
        assert_eq!(
            fmt(src),
            "view schematic \"V\" {\n\
             \x20   include bl_left at (26, 3) ports { west: stop; }\n\
             \x20   include chmsl   at (4, 3) ports { west: stop; }\n\
             \x20   include tail    at (8, 3);\n\
             \x20   include x at (1, 1) ports {\n\
             \x20       west: a;\n\
             \x20       east: b;\n\
             \x20   }\n\
             \x20   include y at (2, 2) ports {\n\
             \x20       west: a;\n\
             \x20   }\n\
             }\n"
        );
    }

    #[test]
    fn an_empty_block_closes_on_its_line() {
        assert_eq!(fmt("component A {\n\n}\n"), "component A {}\n");
        assert_eq!(
            fmt("component A {\n  // later\n}\n"),
            "component A {\n    // later\n}\n"
        );
        assert_eq!(fmt(""), "");
    }

    #[test]
    fn a_source_that_does_not_parse_is_refused() {
        assert!(matches!(
            Formatted::new("component A { pub port a \"A\" }"),
            Err(Unformattable::Parse(errors)) if !errors.is_empty()
        ));
        assert!(matches!(
            Formatted::new("component A { pub port wire \"W\"; }"),
            Err(Unformattable::Parse(_))
        ));
        assert!(matches!(
            Formatted::new("component A { @ }"),
            Err(Unformattable::Lex(_))
        ));
    }

    #[test]
    fn crlf_line_ends_become_newlines() {
        assert_eq!(
            fmt("component A {\r\n    // c\r\n    pub port a \"A\";\r\n}\r\n"),
            "component A {\n    // c\n    pub port a \"A\";\n}\n"
        );
    }
}
