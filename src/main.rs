use std::clone::Clone;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};

use clap::{App, AppSettings, Arg};

#[macro_use]
extern crate anyhow;

use anyhow::{Context, Result};

#[derive(Debug)]
enum Delimiter {
    Whitespace,
    String(String),
}

#[derive(Debug)]
struct FieldSelector {
    fields: Vec<isize>,
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
            fields: vec![s.parse::<isize>()?],
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
            if start < stop {
                Ok((start..=stop).collect::<Vec<isize>>())
            } else {
                Ok((stop..=start).rev().collect::<Vec<isize>>())
            }
        })
        .collect::<Result<Vec<Vec<isize>>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<isize>>();

    Ok(FieldSelector {
        fields: field_indexes,
    })
}

fn parse_command_line<S>(params: Option<Vec<S>>) -> Result<(CutJob, Vec<OsString>)>
where
    S: Into<OsString> + Clone,
{
    let matcher = App::new("rcut")
        .version("1.0")
        .setting(AppSettings::AllowNegativeNumbers)
        .author("Chip Turner <cturner@pattern.net>")
        .about("cut-like tool with smoother aesthetics")
        .arg(
            Arg::with_name("delimiter")
                .short("d")
                .long("delimiter")
                .help("field delimiter")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("output_separator")
                .short("o")
                .long("output_separator")
                .help("separator used when printing fields")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("fields")
                .short("f")
                .long("fields")
                .required(false)
                .help("fields to select")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("args")
                .help("file(s) to process or field selectors")
                .index(1)
                .multiple(true)
                .required(false)
                .takes_value(true),
        );
    let matches = match params {
        Some(p) => matcher.get_matches_from_safe(p)?,
        None => matcher.get_matches_safe()?,
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
    dbg!(&cut_job);
    dbg!(&args);
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
            for field_idx in self.selector.fields.iter() {
                let field_idx = if *field_idx < 0 {
                    line_fields.len() - (-*field_idx as usize) + 1
                } else {
                    *field_idx as usize
                };
                match line_fields.get(field_idx - 1) {
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
            output.write_all(b"\n")?;
        }
        output.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_cli_parsing() -> Result<()> {
        let (cut_job, args) = parse_command_line(Some(vec!["rcut_test", "-f", "1"]))?;
        assert_eq!(cut_job.selector.fields, vec![1]);
        assert_eq!(args, Vec::<OsString>::new());

        let (cut_job, args) = parse_command_line(Some(vec!["rcut_test", "1"]))?;
        assert_eq!(cut_job.selector.fields, vec![1]);
        assert_eq!(args, Vec::<OsString>::new());

        let (cut_job, args) = parse_command_line(Some(vec!["rcut_test", "-1"]))?;
        assert_eq!(cut_job.selector.fields, vec![-1]);
        assert_eq!(args, Vec::<OsString>::new());

        let (cut_job, args) =
            parse_command_line(Some(vec!["rcut_test", "-f", "1", "/etc/passwd"]))?;
        assert_eq!(cut_job.selector.fields, vec![1]);
        assert_eq!(args, vec!["/etc/passwd"]);

        let (cut_job, args) = parse_command_line(Some(vec!["rcut_test", "1-5"]))?;
        assert_eq!(cut_job.selector.fields, vec![1, 2, 3, 4, 5]);
        assert_eq!(args, Vec::<OsString>::new());

        let (cut_job, args) = parse_command_line(Some(vec!["rcut_test", "1-5"]))?;
        assert_eq!(cut_job.selector.fields, vec![1, 2, 3, 4, 5]);
        assert_eq!(args, Vec::<OsString>::new());

        Ok(())
    }

    #[test]
    fn test_simple_field_parsing() -> Result<()> {
        assert_eq!(field_parser("1")?.fields, vec![1]);
        assert_eq!(field_parser("1,2")?.fields, vec![1, 2]);
        assert_eq!(field_parser("1-2,3-4")?.fields, vec![1, 2, 3, 4]);
        assert_eq!(field_parser("2-1,3-4")?.fields, vec![2, 1, 3, 4]);

        Ok(())
    }

    #[test]
    fn test_cut_job() -> Result<()> {
        let input_delim = Delimiter::Whitespace;
        let selector = FieldSelector {
            fields: vec![1, 3, 5],
        };
        let input = BufReader::new("a b c d e f g\np q r s t u\ni j k".as_bytes());
        let mut output = Cursor::new(vec![]);
        let job = CutJob {
            input_delim,
            selector,
            output_separator: " ".to_string(),
        };
        job.process_reader(input, &mut output)?;
        assert_eq!(output.get_ref(), &"a c e\np r t\ni k\n".as_bytes());

        Ok(())
    }
}
