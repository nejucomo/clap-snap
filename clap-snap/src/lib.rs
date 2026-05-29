//! Streamline `clap` and execution flow by deriving subcommands from arg struct methods.
//!
//! ```no_run
//! use clap_snap::App;
//! use std::path::PathBuf;
//!
//! /// An example clap_snap app
//! #[derive(App)]
//! struct MyApp {
//!   /// verbose output
//!   verbose: bool,
//!   /// the config file path
//!   config: PathBuf,
//! }
//!
//! impl MyApp {
//!     #[clap_snap::command]
//!     fn status(&mut self) -> eyre::Result<()> {
//!       if self.verbose {
//!         println!("config path: {}", self.config.display());
//!       }
//!       // print other status info
//!       Ok(())
//!     }
//!
//!     #[clap_snap::command]
//!     fn frobnicate(&mut self, name: String, frobbiness: usize) -> eyre::Result<()> {
//!         let name = name.as_str();
//!         if self.verbose {
//!           println!("frobnicating {name:?} to frobbiness level {frobbiness}");
//!         }
//!         // ... etc...
//!         Ok(())
//!     }
//! }
//! ```

extern crate self as clap_snap;

pub use clap_snap_derive::{App, command};

pub trait App: clap::Parser {
    fn parse_and_run() -> eyre::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::App;
    use clap::Parser;

    #[derive(Debug, Default, App)]
    struct DemoApp {
        verbose: bool,
    }

    impl DemoApp {
        #[crate::command]
        fn status(&mut self) -> eyre::Result<()> {
            self.verbose = true;
            Ok(())
        }

        #[crate::command]
        fn frobnicate(&mut self, _name: String, _frobbiness: usize) -> eyre::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn derive_generates_a_parser() {
        let parsed = DemoApp::try_parse_from(["demo-app", "--verbose", "status"]);
        assert!(parsed.is_ok());
    }
}
