use super::CommandLine;

#[derive(Clone, Debug)]
pub enum CommandLineProgram {
    Main,
    Help,
    Version,
}

impl CommandLineProgram {
    pub fn parse_or_run() -> Option<CommandLine> {
        let cmd = CommandLine::parse();

        match cmd.program {
            CommandLineProgram::Main => return Some(cmd),
            CommandLineProgram::Help => {
                println!("{}", include_str!("usage.txt"))
            }
            CommandLineProgram::Version => {
                println!("tetron-browser {} (Carbonyl 0.0.3)", env!("CARGO_PKG_VERSION"))
            }
        }

        None
    }
}
