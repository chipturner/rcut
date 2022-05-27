use std::{
    clone::Clone,
    ffi::OsString,
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Write},
};

use clap::{Arg, Command};

#[macro_use]
extern crate anyhow;

use anyhow::{Context, Result};

#[derive(Debug)]
enum Delimiter {
    Whitespace,
    String(String),
}

#[derive(Debug, PartialEq, Eq)]
struct FieldRange {
    start: isize,
    stop: isize,
}

impl FieldRange {
    fn new_span(start: isize, stop: isize) -> Self {
        FieldRange { start, stop }
    }
    fn new_val(val: isize) -> Self {
        FieldRange {
            start: val,
            stop: val,
        }
    }
}

#[derive(Debug)]
struct FieldSelector {
    fields: Vec<FieldRange>,
}

#[derive(Debug)]
struct CutJob {
    input_delim: Delimiter,
    selector: FieldSelector,
    output_separator: String,
}

fn field_parser<S: Into<String>>(s: S) -> Result<FieldSelector> {
    let s = s.into();
    if s.starts_with('-') {
        return Ok(FieldSelector {
            fields: vec![FieldRange::new_val(s.parse::<isize>()?)],
        });
    }
    let field_indexes = s
        .split(',')
        .map(|t| {
            let mut ranges = t.splitn(2, '-').map(|s| s.parse::<isize>());
            let start = ranges
                .next()
                .ok_or_else(|| format_err!("empty field range"))??;
            let stop = ranges.next().unwrap_or(Ok(start))?;
            Ok(FieldRange::new_span(start, stop))
        })
        .collect::<Result<Vec<FieldRange>>>()?;

    Ok(FieldSelector {
        fields: field_indexes,
    })
}

fn parse_command_line<S>(params: Option<Vec<S>>) -> Result<(CutJob, Vec<OsString>)>
where
    S: Into<OsString> + Clone + std::fmt::Debug,
{
    let matcher = Command::new("rcut")
        .version("1.0")
        .author("Chip Turner <cturner@pattern.net>")
        .about("cut-like tool with smoother aesthetics")
        .arg(
            Arg::new("delimiter")
                .short('d')
                .multiple_occurrences(false)
                .help("field delimiter")
                .takes_value(true),
        )
        .arg(
            Arg::new("output_separator")
                .short('o')
                .multiple_occurrences(false)
                .help("separator used when printing fields")
                .takes_value(true),
        )
        .arg(
            Arg::new("fields")
                .short('f')
                .help("fields to select")
                .multiple_occurrences(false)
                .takes_value(true),
        )
        .arg(
            Arg::new("args")
                .help("file(s) to process or field selectors")
                .multiple_occurrences(true)
                .required(false)
                .takes_value(true)
                .index(1)
                .allow_invalid_utf8(true),
        );

    let matches = match params {
        Some(p) => matcher.try_get_matches_from(p)?,
        None => matcher.try_get_matches()?,
    };
    let args: Vec<OsString> = match matches.values_of_os("args") {
        Some(vals) => vals.map(OsString::from).collect(),
        None => vec![],
    };

    let (selector, args) = if matches.is_present("fields") {
        (
            field_parser(String::from(matches.value_of("fields").unwrap())),
            args,
        )
    } else {
        (
            field_parser(
                args.iter()
                    .map(|s| s.to_str().unwrap())
                    .collect::<Vec<&str>>()
                    .join(","),
            ),
            vec![],
        )
    };
    let selector = selector?;

    let input_delim = matches
        .value_of("delimiter")
        .map_or(Delimiter::Whitespace, |v| {
            Delimiter::String(String::from(v))
        });

    let output_separator = String::from(
        matches
            .value_of("output_separator")
            .unwrap_or_else(|| matches.value_of("delimiter").unwrap_or(" ")),
    );

    let cut_job = CutJob {
        input_delim,
        selector,
        output_separator,
    };

    Ok((cut_job, args))
}

fn main() -> Result<()> {
    let (cut_job, args) = parse_command_line::<OsString>(None)?;
    let stdout = io::stdout();
    let mut stdout = BufWriter::new(stdout.lock());

    if !args.is_empty() {
        if let Err(err) = args
            .iter()
            .map(|filename| {
                File::open(filename).with_context(|| filename.to_string_lossy().into_owned())
            })
            .map(|result| result.map(|fh| Box::new(BufReader::new(fh))))
            .map(|result| result.and_then(|val| cut_job.process_reader(val, &mut stdout)))
            .collect::<Result<Vec<()>>>()
        {
            muffle_epipe(err)?;
        }
    } else {
        let stdin = io::stdin();
        if let Err(err) = cut_job.process_reader(stdin.lock(), &mut stdout) {
            muffle_epipe(err)?;
        }
    }
    Ok(())
}

// If err is actually a EPIPE, pretend things are fine; otherwise
// propagate error.  This way if stdout is closed (say, part of a
// pipeline) we still exit cleanly, like `cut`.
fn muffle_epipe(err: anyhow::Error) -> Result<()> {
    for cause in err.chain() {
        if let Some(io_err) = cause.downcast_ref::<io::Error>() {
            if io_err.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
        }
    }
    Err(err)
}

impl CutJob {
    // Read a stream, splitting each line on the Delimiter and outputting
    // as requested by the field Selector.
    fn process_reader(&self, reader: impl BufRead, output: &mut impl Write) -> Result<()> {
        for line in reader.lines() {
            let line = line?;
            let line_fields: Vec<&str> = match self.input_delim {
                Delimiter::String(ref s) => line.split(s.as_str()).collect(),
                Delimiter::Whitespace => line.split_whitespace().collect(),
            };

            let mut needs_sep = false;
            for range in self.selector.fields.iter() {
                for idx in range.start..=range.stop {
                    let idx = if idx < 0 {
                        line_fields.len() as isize - -idx + 1
                    } else {
                        idx
                    };
                    match line_fields.get((idx - 1) as usize) {
                        None => continue,
                        Some(val) => {
                            if needs_sep {
                                output.write_all(self.output_separator.as_bytes())?;
                            }
                            output.write_all(val.as_bytes())?;
                            needs_sep = true;
                        }
                    }
                }
            }
            output.write_all(b"\n")?;
        }
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn test_cli_parsing() {
        let (cut_job, args) = parse_command_line(Some(vec!["rcut_test", "-f", "1"])).unwrap();
        assert_eq!(cut_job.selector.fields, vec![FieldRange::new_val(1)]);
        assert_eq!(args, Vec::<OsString>::new());

        let (cut_job, args) = parse_command_line(Some(vec!["rcut_test", "1"])).unwrap();
        assert_eq!(cut_job.selector.fields, vec![FieldRange::new_val(1)]);
        assert_eq!(args, Vec::<OsString>::new());

        let (cut_job, args) =
            parse_command_line(Some(vec!["rcut_test", "-f", "1", "/etc/passwd"])).unwrap();
        assert_eq!(cut_job.selector.fields, vec![FieldRange::new_val(1)]);
        assert_eq!(args, vec!["/etc/passwd"]);

        let (cut_job, args) = parse_command_line(Some(vec!["rcut_test", "1-5"])).unwrap();
        assert_eq!(cut_job.selector.fields, vec![FieldRange::new_span(1, 5)]);
        assert_eq!(args, Vec::<OsString>::new());

        let (cut_job, args) = parse_command_line(Some(vec!["rcut_test", "1-5"])).unwrap();
        assert_eq!(cut_job.selector.fields, vec![FieldRange::new_span(1, 5)]);
        assert_eq!(args, Vec::<OsString>::new());
    }

    #[test]
    fn test_simple_field_parsing() {
        assert_eq!(FieldRange::new_val(1), FieldRange::new_span(1, 1));
        assert_eq!(
            field_parser("1").unwrap().fields,
            vec![FieldRange::new_val(1)]
        );
        assert_eq!(
            field_parser("1,2").unwrap().fields,
            vec![FieldRange::new_val(1), FieldRange::new_val(2)]
        );
        assert_eq!(
            field_parser("1-1").unwrap().fields,
            vec![FieldRange::new_val(1)]
        );
        assert_eq!(
            field_parser("1-4").unwrap().fields,
            vec![FieldRange::new_span(1, 4)]
        );
        assert_eq!(
            field_parser("1-2,3-4").unwrap().fields,
            vec![FieldRange::new_span(1, 2), FieldRange::new_span(3, 4)]
        );
        assert_eq!(
            field_parser("2-1,3-4").unwrap().fields,
            vec![FieldRange::new_span(2, 1), FieldRange::new_span(3, 4)]
        );
    }

    fn exec_cut_job(job: CutJob, input: &str) -> Result<String> {
        let input = BufReader::new(input.as_bytes());
        let mut output = Cursor::new(vec![]);
        job.process_reader(input, &mut output).unwrap();
        Ok(String::from_utf8(output.get_ref().to_vec()).unwrap())
    }

    #[test]
    fn test_cut_job() {
        let simple_alphabet = "a b c d e f g\np q r s t u\ni j k\n";
        let job = CutJob {
            input_delim: Delimiter::Whitespace,
            selector: field_parser("-1").unwrap(),
            output_separator: " ".to_string(),
        };
        assert_eq!(exec_cut_job(job, simple_alphabet).unwrap(), "g\nu\nk\n");

        let job = CutJob {
            input_delim: Delimiter::Whitespace,
            selector: field_parser("1-3").unwrap(),
            output_separator: " ".to_string(),
        };
        assert_eq!(
            exec_cut_job(job, simple_alphabet).unwrap(),
            "a b c\np q r\ni j k\n"
        );
    }
}
