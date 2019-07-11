use std::io::{self, BufRead, BufWriter, Write};

extern crate clap;
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
    let v = s
        .split(',')
        .map(|t| {
            let ranges = t
                .split('-')
                .map(|s| s.parse::<usize>())
                .collect::<std::result::Result<Vec<usize>, _>>()?;
            if ranges.len() == 1 {
                Ok((ranges[0]..=ranges[0]).collect::<Vec<usize>>())
            } else if ranges.len() == 2 {
                if ranges[1] < ranges[0] {
                    Ok((ranges[1]..=ranges[0]).collect::<Vec<usize>>())
                } else {
                    Ok((ranges[0]..=ranges[1]).collect::<Vec<usize>>())
                }
            } else {
                Err(format_err!("invalid range: {:?}", ranges))
            }
        })
        .collect::<Result<Vec<Vec<usize>>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<usize>>();

    Ok(FieldSelector { fields: v })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_field() {
        assert_eq!(field_parser("1").unwrap().fields, vec![1]);
        assert_eq!(field_parser("1,2").unwrap().fields, vec![1, 2]);
        assert_eq!(field_parser("1-2,3-4").unwrap().fields, vec![1, 2, 3, 4]);
        assert_eq!(field_parser("2-1,3-4").unwrap().fields, vec![1, 2, 3, 4]);
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
                .index(1)
                .required(true)
                .help("fields to select")
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
    for line in stdin.lock().lines() {
        let line = line?;
        let line_fields: Vec<&str> = match delim {
            Delimiter::String(ref s) => line.split(s.as_str()).collect(),
            Delimiter::Whitespace => line.split_whitespace().collect(),
        };

        for (idx, field_idx) in selector.fields.iter().enumerate() {
            stdout.write_all(line_fields[*field_idx].as_bytes())?;
            if idx < selector.fields.len() - 1 {
                stdout.write_all(b":")?;
            }
        }
        stdout.write_all(b"\n")?;
    }
    Ok(())
}
