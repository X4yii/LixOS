pub trait EarlyConsole: Send {
    fn write_str(&mut self, s: &str);
}
