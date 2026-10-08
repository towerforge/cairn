//! The version numbers Cairn publishes: `MAJOR.MINOR.PATCH` with an optional
//! pre-release tag. Too little to be worth a dependency, and the order is the
//! only thing the updater asks of it.

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// What follows the `-`: `rc.1`, `beta`… A pre-release comes before the
    /// release it leads to, as semver says.
    pub pre: Option<String>,
}

impl Version {
    /// The version of the running binary.
    pub fn current() -> Self {
        env!("CARGO_PKG_VERSION")
            .parse()
            .expect("CARGO_PKG_VERSION is semver")
    }

    /// The tag of the release that carries this version: `v0.4.0`.
    pub fn tag(&self) -> String {
        format!("v{self}")
    }
}

impl FromStr for Version {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || anyhow::anyhow!("`{s}` is not a version like 1.2.3");
        let t = s.trim();
        let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
        // Build metadata (`+abc`) plays no part in the order: drop it.
        let t = t.split('+').next().unwrap_or(t);
        let (core, pre) = match t.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
            Some(_) => return Err(bad()),
            None => (t, None),
        };
        let mut it = core.split('.');
        let mut num = || -> anyhow::Result<u64> {
            it.next()
                .filter(|p| !p.is_empty())
                .ok_or_else(bad)?
                .parse()
                .map_err(|_| bad())
        };
        let (major, minor, patch) = (num()?, num()?, num()?);
        if it.next().is_some() {
            return Err(bad());
        }
        Ok(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        match &self.pre {
            Some(p) => write!(f, "-{p}"),
            None => Ok(()),
        }
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                // 1.0.0 is above 1.0.0-rc.1
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => cmp_pre(a, b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Pre-release tags, identifier by identifier: numbers as numbers, the rest
/// as text, and a number below anything alphanumeric.
fn cmp_pre(a: &str, b: &str) -> Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return Ordering::Equal,
            // The one that runs out of identifiers first is the lower one.
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(nx), Ok(ny)) => nx.cmp(&ny),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        s.parse().expect("valid version")
    }

    #[test]
    fn accepts_the_v_from_the_tag() {
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        assert_eq!(v("  v0.1.1 ").to_string(), "0.1.1");
        assert_eq!(v("1.2.3+abc"), v("1.2.3"));
    }

    #[test]
    fn rejects_what_is_not_a_version() {
        for s in ["", "1.2", "1.2.3.4", "1.2.x", "v", "1.2.3-", "abc"] {
            assert!(s.parse::<Version>().is_err(), "{s} should not parse");
        }
    }

    #[test]
    fn orders_by_number_and_puts_pre_releases_first() {
        assert!(v("0.2.0") > v("0.1.9"));
        assert!(v("1.0.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.2"));
        assert!(v("1.0.0-beta") < v("1.0.0-rc"));
    }

    #[test]
    fn prints_as_it_parses() {
        for s in ["0.1.1", "1.2.3-rc.1"] {
            assert_eq!(v(s).to_string(), s);
        }
        assert_eq!(v("0.4.0").tag(), "v0.4.0");
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }
}
