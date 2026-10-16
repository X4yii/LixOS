use crate::{AuditEvent, AuditLog};
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
pub trait AuditSink {
    fn emit(&mut self, line: &str);
}
pub struct LineAudit<S> {
    pub sink: S,
}
impl<S: AuditSink> AuditLog for LineAudit<S> {
    fn write(&mut self, event: AuditEvent) {
        let line = render(&event);
        self.sink.emit(&line);
    }
}
pub struct MemoryAudit {
    pub lines: Vec<String>,
}
impl AuditLog for MemoryAudit {
    fn write(&mut self, event: AuditEvent) {
        self.lines.push(render(&event));
    }
}
pub fn render(event: &AuditEvent) -> String {
    match event {
        AuditEvent::Login { user, granted } => {
            String::from("session user=") + &user.to_string() + flag(*granted)
        }
        AuditEvent::Enrollment {
            user,
            administrator,
        } => {
            String::from("enroll user=")
                + &user.to_string()
                + " administrator="
                + bool_word(*administrator)
        }
        AuditEvent::Elevation {
            user,
            program,
            scope,
            granted,
        } => {
            String::from("elevation user=")
                + &user.to_string()
                + " program="
                + &program.to_string()
                + " scope="
                + scope_name(*scope)
                + flag(*granted)
        }
    }
}
const fn flag(granted: bool) -> &'static str {
    if granted {
        " granted=true"
    } else {
        " granted=false"
    }
}
const fn bool_word(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}
const fn scope_name(scope: crate::Scope) -> &'static str {
    match scope {
        crate::Scope::SystemConfiguration => "system-configuration",
        crate::Scope::NetworkAdministration => "network-administration",
        crate::Scope::BluetoothAdministration => "bluetooth-administration",
        crate::Scope::PowerManagement => "power-management",
        crate::Scope::AdministrativeSession => "administrative-session",
    }
}
