use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};

use clap::{App, Arg};

#[macro_use]
extern crate failure;

type Result<T> = ::std::result::Result<T, failure::Error>;

enum Delimiter {
    Whitespace,
    String(String),
}

struct FieldSelector {
    fields: Vec<usize>,
}

fn field_parser(s: &str) -> Result<FieldSelector> {
    let field_indexes = s
        .split(',')
        .map(|t| {
            let mut ranges = t.splitn(2, '-').map(|s| s.parse::<usize>());
            let start = ranges
                .next()
                .ok_or_else(|| format_err!("empty field range"))??;
            let stop = ranges.next().unwrap_or(Ok(start))?;
            if start < stop {
                Ok((start..=stop).collect::<Vec<usize>>())
            } else {
                Ok((stop..=start).rev().collect::<Vec<usize>>())
            }
        })
        .collect::<Result<Vec<Vec<usize>>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<usize>>();

    Ok(FieldSelector {
        fields: field_indexes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_field() {
        assert_eq!(field_parser("1").unwrap().fields, vec![1]);
        assert_eq!(field_parser("1,2").unwrap().fields, vec![1, 2]);
        assert_eq!(field_parser("1-2,3-4").unwrap().fields, vec![1, 2, 3, 4]);
        assert_eq!(field_parser("2-1,3-4").unwrap().fields, vec![2, 1, 3, 4]);
    }
}

fn main() -> Result<()> {
    let matches = App::new("rcut")
        .version("1.0")
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
            Arg::with_name("fields")
                .short("f")
                .long("fields")
                .required(true)
                .help("fields to select")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("input")
                .help("file(s) to process (otherwise read from stdin)")
                .index(1)
                .multiple(true)
                .required(false)
                .takes_value(true),
        )
        .get_matches();

    let delim = matches
        .value_of("delimiter")
        .map_or(Delimiter::Whitespace, |v| {
            Delimiter::String(String::from(v))
        });
    let selector = field_parser(matches.value_of("fields").unwrap_or("1"))?;

    let stdout = io::stdout();
    let mut stdout = BufWriter::new(stdout.lock());
    let stdin = io::stdin();

    let input: Vec<Box<dyn BufRead>> = match matches.values_of("input") {
        None => vec![Box::new(stdin.lock())],
        Some(inputs) => inputs
            .map(|v| -> Box<dyn BufRead> { Box::new(BufReader::new(File::open(v).unwrap())) })
            .collect(),
    };
    for reader in input {
        for line in reader.lines() {
            let line = line?;
            let line_fields: Vec<&str> = match delim {
                Delimiter::String(ref s) => line.split(s.as_str()).collect(),
                Delimiter::Whitespace => line.split_whitespace().collect(),
            };

            for (idx, field_idx) in selector.fields.iter().enumerate() {
                match line_fields.get(*field_idx - 1) {
                    None => continue,
                    Some(val) => stdout.write_all(val.as_bytes())?,
                }
                if idx < selector.fields.len() - 1 {
                    stdout.write_all(b":")?;
                }
            }
            stdout.write_all(b"\n")?;
        }
    }
    Ok(())
}
