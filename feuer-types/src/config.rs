//! Shared parsing and environment loading for numeric configuration settings.

use std::ffi::OsStr;

/// Parses a number with optional `bytesize` suffixes, checking its minimum and integer type.
/// Suffixes are multipliers even when the setting represents a count rather than bytes.
pub fn parse_config_number<T: TryFrom<u64>>(value: &OsStr, minimum: u64) -> Option<T> {
    let number = value.to_str()?.parse::<bytesize::ByteSize>().ok()?.as_u64();
    if number < minimum {
        return None;
    }
    T::try_from(number).ok()
}

/// Reads a numeric environment setting, using the default only when it is unset.
/// Invalid settings return an error naming the variable and its required range.
pub fn read_env_number<T: TryFrom<u64>>(name: &str, default: T, minimum: u64) -> Result<T, String> {
    match std::env::var_os(name) {
        None => Ok(default),
        Some(value) => parse_config_number(&value, minimum).ok_or_else(|| {
            format!(
                "{name} must be a number >= {minimum} fitting {} (e.g. 8192 or 8KiB)",
                std::any::type_name::<T>()
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_scaled_numbers() {
        for (input, expected) in [
            ("0", 0),
            ("8192", 8192),
            ("18446744073709551615", u64::MAX),
            ("8KiB", 8192),
            ("512MiB", 512 << 20),
            ("5GiB", 5 << 30),
            ("1TiB", 1 << 40),
            ("1.5 GiB", 3 << 29),
            ("1KB", 1000),
            ("2mb", 2_000_000),
            ("5GB", 5_000_000_000),
        ] {
            assert_eq!(parse_config_number(OsStr::new(input), 0), Some(expected), "{input}");
        }
    }

    #[test]
    fn rejects_invalid_numbers() {
        for input in ["", "-1", "abc", "1.5", " 64", "64 ", "18446744073709551616"] {
            assert_eq!(parse_config_number::<u64>(OsStr::new(input), 0), None, "{input}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert_eq!(parse_config_number::<u64>(OsStr::from_bytes(b"\xff"), 0), None);
        }
    }

    #[test]
    fn checks_minimum_and_target_integer_range() {
        assert_eq!(parse_config_number::<usize>(OsStr::new("0"), 0), Some(0));
        assert_eq!(parse_config_number::<usize>(OsStr::new("0"), 1), None);
        assert_eq!(parse_config_number::<usize>(OsStr::new("1KiB"), 1), Some(1024));
        assert_eq!(parse_config_number::<u8>(OsStr::new("256"), 0), None);
        assert_eq!(parse_config_number::<u8>(OsStr::new("255"), 1), Some(255));
        assert_eq!(
            parse_config_number::<usize>(OsStr::new(&usize::MAX.to_string()), 1),
            Some(usize::MAX)
        );
    }

    #[test]
    fn reads_environment_without_mutating_other_tests() {
        const NAME: &str = "FEUER_TEST_CONFIG_NUMBER";
        const CHILD: &str = "FEUER_TEST_CONFIG_NUMBER_CHILD";
        if let Ok(expected) = std::env::var(CHILD) {
            let result = read_env_number::<usize>(NAME, 64, 1);
            if expected == "error" {
                let error = result.unwrap_err();
                assert!(error.contains(NAME));
                assert!(error.contains(">= 1 fitting usize"));
            } else {
                assert_eq!(result.unwrap(), expected.parse::<usize>().unwrap());
            }
            return;
        }
        for (value, expected) in [
            (None, "64"),
            (Some("8KiB"), "8192"),
            (Some("0"), "error"),
            (Some("bad"), "error"),
        ] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child.args([
                "--exact",
                "config::tests::reads_environment_without_mutating_other_tests",
            ]);
            child.env(CHILD, expected).env_remove(NAME);
            if let Some(value) = value {
                child.env(NAME, value);
            }
            let output = child.output().unwrap();
            assert!(output.status.success(), "{output:?}");
        }
    }
}
