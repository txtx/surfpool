#[allow(unused_macros)]
#[macro_export]
macro_rules! green {
    ($($arg:tt)*) => (
        {
            use atty::Stream;
            use ansi_term::Colour;
            if atty::is(Stream::Stdout) {
                let colour = Colour::Green.bold();
                format!(
                    "{}",
                    colour.paint($($arg)*)
                )
            } else {
                format!(
                    "{}",
                    $($arg)*
                )
            }
        }
    )
}

#[allow(unused_macros)]
#[macro_export]
macro_rules! red {
    ($($arg:tt)*) => (
        {
            use atty::Stream;
            use ansi_term::Colour;
            if atty::is(Stream::Stdout) {
                let colour = Colour::Red.bold();
                format!(
                    "{}",
                    colour.paint($($arg)*)
                )
            } else {
                format!(
                    "{}",
                    $($arg)*
                )
            }
        }
    )
}

#[allow(unused_macros)]
#[macro_export]
macro_rules! yellow {
    ($($arg:tt)*) => (
        {
            use atty::Stream;
            use ansi_term::Colour;
            if atty::is(Stream::Stdout) {
                let colour = Colour::Yellow.bold();
                format!(
                    "{}",
                    colour.paint($($arg)*)
                )
            } else {
                format!(
                    "{}",
                    $($arg)*
                )
            }
        }
    )
}

#[allow(unused_macros)]
#[macro_export]
macro_rules! blue {
    ($($arg:tt)*) => (
        {
            use atty::Stream;
            use ansi_term::Colour;
            if atty::is(Stream::Stdout) {
                let colour = Colour::Cyan.bold();
                format!(
                    "{}",
                    colour.paint($($arg)*)
                )
            } else {
                format!(
                    "{}",
                    $($arg)*
                )
            }
        }
    )
}

#[allow(unused_macros)]
#[macro_export]
macro_rules! purple {
    ($($arg:tt)*) => (
        {
            use atty::Stream;
            use ansi_term::Colour;
            if atty::is(Stream::Stdout) {
                let colour = Colour::Purple.bold();
                format!(
                    "{}",
                    colour.paint($($arg)*)
                )
            } else {
                format!(
                    "{}",
                    $($arg)*
                )
            }
        }
    )
}

#[allow(unused_macros)]
#[macro_export]
macro_rules! black {
    ($($arg:tt)*) => (
        {
            use atty::Stream;
            use ansi_term::Colour;
            if atty::is(Stream::Stdout) {
                let colour = Colour::Fixed(244);
                format!(
                    "{}",
                    colour.paint($($arg)*)
                )
            } else {
                format!(
                    "{}",
                    $($arg)*
                )
            }
        }
    )
}

#[macro_export]
macro_rules! pluralize {
    ($value:expr, $word:expr) => {
        if $value > 1 {
            format!("{} {}s", $value, $word)
        } else {
            format!("{} {}", $value, $word)
        }
    };
}

#[allow(unused_macros)]
#[macro_export]
macro_rules! format_err {
    ($($arg:tt)*) => (
        {
            format!("{} {}", red!("error:"), $($arg)*)
        }
    )
}

#[allow(unused_macros)]
#[macro_export]
macro_rules! format_warn {
    ($($arg:tt)*) => (
        {
            format!("{} {}", yellow!("warn:"), $($arg)*)
        }
    )
}

#[allow(unused_macros)]
#[macro_export]
macro_rules! format_note {
    ($($arg:tt)*) => (
        {
            format!("{} {}", blue!("note:"), $($arg)*)
        }
    )
}

// Created macro to compare fields of snapshot and mainnet accounts
#[allow(unused_macros)]
#[macro_export]
macro_rules! compare_field {
    ($field_name:expr,$snapshot_val:expr,$mainnet_val:expr) => {
        if $snapshot_val != $mainnet_val {
            println!("    {} differs: Snapshot={}, Mainnet={}", 
                     $field_name, $snapshot_val, $mainnet_val);
            false
        } else {
            true
        }
    };
}