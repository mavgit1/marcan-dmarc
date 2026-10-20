//! CSV writing, through the `csv` crate, for the exports (CRLF lines, as RFC
//! 4180 has them) and the list texts in Settings (LF lines). A field is quoted
//! only when it needs it (a comma, a quote or a line break), inner quotes
//! doubled.

/// How a CSV line ends.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LineEnd {
    /// `\r\n`, for the downloads under `/api/export/`.
    Crlf,
    /// `\n`, for the text a person edits (the domain mapping, the client
    /// report recipients).
    Lf,
}

/// Append one CSV line of `fields` to `out`.
pub(crate) fn write_line<I, T>(out: &mut Vec<u8>, end: LineEnd, fields: I)
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let terminator = match end {
        LineEnd::Crlf => csv::Terminator::CRLF,
        LineEnd::Lf => csv::Terminator::Any(b'\n'),
    };
    let mut writer = csv::WriterBuilder::new()
        .terminator(terminator)
        .buffer_capacity(1024)
        .from_writer(out);
    for field in fields {
        writer
            .write_field(field.as_ref())
            .expect("writing to memory");
    }
    // No more fields: just the line end.
    writer
        .write_record(None::<&[u8]>)
        .expect("writing to memory");
    writer.flush().expect("writing to memory");
}

/// `lines` as LF-terminated CSV text; lines may differ in length.
pub(crate) fn lf_text<L, I, T>(lines: L) -> String
where
    L: IntoIterator<Item = I>,
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let mut out = Vec::new();
    for fields in lines {
        write_line(&mut out, LineEnd::Lf, fields);
    }
    String::from_utf8(out).expect("UTF-8 in, UTF-8 out")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(end: LineEnd, fields: &[&str]) -> String {
        let mut out = Vec::new();
        write_line(&mut out, end, fields);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn fields_are_quoted_only_when_needed() {
        assert_eq!(line(LineEnd::Crlf, &["a", "b c", ""]), "a,b c,\r\n");
        assert_eq!(
            line(LineEnd::Crlf, &["x,y", "say \"hi\"", "two\nlines", "a\rb"]),
            "\"x,y\",\"say \"\"hi\"\"\",\"two\nlines\",\"a\rb\"\r\n"
        );
        assert_eq!(
            line(LineEnd::Lf, &["Globex, Inc", "a@b"]),
            "\"Globex, Inc\",a@b\n"
        );
    }

    #[test]
    fn lines_may_differ_in_length() {
        assert_eq!(
            lf_text([vec!["a", "b"], vec!["c", "d", "parked"]]),
            "a,b\nc,d,parked\n"
        );
    }
}
