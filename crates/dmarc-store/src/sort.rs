//! Server-side sorting for paginated lists: `sort=key` ascending, `sort=-key`
//! descending. Each list defines its own key enum mapped to a fixed SQL column,
//! so user input never reaches the query text.

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub(crate) fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// A list's sortable columns.
pub trait SortKey: Copy + Sized {
    /// Parse the public name of a key (the part after an optional `-`).
    fn parse(name: &str) -> Option<Self>;
    /// SQL column or alias this key orders by.
    fn column(self) -> &'static str;
}

/// A key and a direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort<K> {
    pub key: K,
    pub dir: SortDir,
}

impl<K: SortKey> Sort<K> {
    pub const fn new(key: K, dir: SortDir) -> Self {
        Self { key, dir }
    }

    /// `key` or `-key`; `None` for an unknown key.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        let (name, dir) = match raw.strip_prefix('-') {
            Some(name) => (name, SortDir::Desc),
            None => (raw, SortDir::Asc),
        };
        K::parse(name).map(|key| Self { key, dir })
    }

    /// `column DIR` for an `ORDER BY` clause.
    pub(crate) fn order_by(self) -> String {
        format!("{} {}", self.key.column(), self.dir.sql())
    }
}

/// Declare a sort key enum whose public names equal their SQL column/alias.
macro_rules! sort_keys {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $column:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant),+
        }

        impl $crate::sort::SortKey for $name {
            fn parse(name: &str) -> Option<Self> {
                match name {
                    $($column => Some(Self::$variant),)+
                    _ => None,
                }
            }

            fn column(self) -> &'static str {
                match self {
                    $(Self::$variant => $column,)+
                }
            }
        }
    };
}
pub(crate) use sort_keys;

#[cfg(test)]
mod tests {
    use super::*;

    sort_keys!(TestKey { Count => "count", Ip => "source_ip" });

    #[test]
    fn parses_direction_prefix() {
        assert_eq!(
            Sort::<TestKey>::parse("-count"),
            Some(Sort::new(TestKey::Count, SortDir::Desc))
        );
        assert_eq!(
            Sort::<TestKey>::parse("source_ip"),
            Some(Sort::new(TestKey::Ip, SortDir::Asc))
        );
        assert_eq!(Sort::<TestKey>::parse("bogus"), None);
        assert_eq!(
            Sort::new(TestKey::Ip, SortDir::Desc).order_by(),
            "source_ip DESC"
        );
    }
}
