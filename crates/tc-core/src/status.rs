//! The status of one pane, decided from several signals (PRD section 8): the PTY's foreground
//! process, agent hooks, OSC notifications and the bell, shell integration marks, the user's
//! own input, and the shell's exit. Every status records where it came from; without a
//! trustworthy signal it is `Unknown`, never a guess.

use std::time::{Duration, Instant};

use crate::agent::AgentKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The shell waits at its prompt, or an agent waits for the next prompt.
    Idle,
    /// An agent or a command is working.
    Running,
    /// An agent asks for permission or an answer.
    NeedsInput,
    /// A process ended with a non-zero code.
    Failed(i32),
    /// An agent finished its turn and the user has not looked yet.
    Done,
    /// A long command finished and the user has not looked yet.
    CommandDone,
    /// An agent runs but nothing says what it is doing.
    Unknown,
    /// The pane's shell exited normally.
    Ended,
}

impl Status {
    /// Position in the attention queue (lower first); `None` for statuses that never need the
    /// user.
    pub fn priority(self) -> Option<u8> {
        match self {
            Status::NeedsInput => Some(0),
            Status::Failed(_) => Some(1),
            Status::Done => Some(2),
            Status::CommandDone => Some(3),
            _ => None,
        }
    }
}

/// Where a status came from (shown in the tooltip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    None,
    /// The foreground process or the program's exit.
    Process,
    Hook(AgentKind),
    /// An OSC 9/99/777 notification.
    Osc,
    Bell,
    /// Shell integration (OSC 133).
    Shell,
    /// The user's own input in the pane.
    Input,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Foreground {
    /// The shell itself (at its prompt).
    Shell,
    /// A job in the foreground.
    Program {
        name: String,
        agent: Option<AgentKind>,
    },
    /// The program on the PTY has ended.
    Gone,
}

/// What an agent's hook said, after the per-agent mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookSignal {
    SessionStart,
    Working,
    NeedsInput,
    TurnDone,
    SessionEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellMark {
    /// OSC 133;C: a command was entered.
    CommandStart,
    /// OSC 133;D: it finished (with its exit code when the shell reports one).
    CommandFinished(Option<i32>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Foreground(Foreground),
    Hook {
        agent: AgentKind,
        signal: HookSignal,
        message: Option<String>,
    },
    /// An OSC desktop-notification request.
    Notify { message: Option<String> },
    Bell,
    Shell(ShellMark),
    /// The user pressed Enter in the pane.
    Enter,
    /// The pane's program (the shell) exited.
    Exited(Option<i32>),
}

/// A command that ran at least this long is worth a "finished" notice.
pub const LONG_COMMAND: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct Tracker {
    pub status: Status,
    pub source: Source,
    /// When the status last changed.
    pub since: Instant,
    /// The status wants the user's attention and they have not looked at the pane since.
    pub unseen: bool,
    /// Short text from the signal (for example the agent's permission question).
    pub message: Option<String>,
    pub fg: Foreground,
    fg_since: Instant,
    /// The agent in the foreground, when there is one.
    pub agent: Option<AgentKind>,
    /// Hooks arrived from the current agent run.
    pub hooks_active: bool,
    /// Shell integration marks were seen in this pane.
    pub shell_marks: bool,
    command_started: Option<Instant>,
    /// The user is looking at this pane (focused, window active).
    focused: bool,
    /// Commands at least this long get a "finished" notice (settings, default `LONG_COMMAND`).
    pub long_command: Duration,
    /// Running only because Enter was pressed: unless the agent confirms it (a hook) by this
    /// time, the status goes back to what it was (Enter on an empty line, `/help`, a paste).
    provisional: Option<(Instant, Status)>,
}

/// How long Running-after-Enter waits for the agent to confirm it.
pub const ENTER_CONFIRM: Duration = Duration::from_secs(8);

impl Tracker {
    pub fn new(now: Instant) -> Self {
        Self {
            status: Status::Idle,
            source: Source::None,
            since: now,
            unseen: false,
            message: None,
            fg: Foreground::Shell,
            fg_since: now,
            agent: None,
            hooks_active: false,
            shell_marks: false,
            command_started: None,
            focused: false,
            long_command: LONG_COMMAND,
            provisional: None,
        }
    }

    /// When a provisional status must be checked (see [`Tracker::expire`]).
    pub fn deadline(&self) -> Option<Instant> {
        self.provisional.map(|(at, _)| at)
    }

    /// Ends a Running that Enter set and nothing confirmed. Returns whether the status changed.
    pub fn expire(&mut self, now: Instant) -> bool {
        match self.provisional {
            Some((at, back)) if now >= at => {
                self.provisional = None;
                if self.status == Status::Running && self.source == Source::Input {
                    return self.set(back, Source::Input, false, now);
                }
                false
            }
            _ => false,
        }
    }

    /// Whether the pane belongs in the attention queue.
    pub fn wants_attention(&self) -> bool {
        self.unseen && self.status.priority().is_some()
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// How long the foreground (the shell, or a program) has been what it is.
    pub fn fg_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.fg_since)
    }

    fn set(&mut self, status: Status, source: Source, attention: bool, now: Instant) -> bool {
        let changed = status != self.status || source != self.source || attention != self.unseen;
        if status != self.status {
            self.since = now;
            if status != Status::NeedsInput {
                self.message = None;
            }
        }
        self.status = status;
        self.source = source;
        // Nothing to bring to the user's attention while they look at the pane.
        self.unseen = attention && !self.focused;
        changed
    }

    /// The user starts or stops looking at the pane. Looking clears the attention mark; a
    /// finished turn or command then simply reads as idle.
    pub fn set_focused(&mut self, focused: bool, now: Instant) -> bool {
        let was = self.focused;
        self.focused = focused;
        if focused && self.unseen {
            self.unseen = false;
            if matches!(self.status, Status::Done | Status::CommandDone) {
                self.status = Status::Idle;
                self.since = now;
            }
            return true;
        }
        was != focused
    }

    /// Applies one signal. Returns whether anything visible changed.
    pub fn apply(&mut self, event: Event, now: Instant) -> bool {
        // Anything the agent or the shell says settles a provisional Running.
        if !matches!(event, Event::Enter | Event::Foreground(_)) {
            self.provisional = None;
        }
        match event {
            Event::Foreground(fg) => self.foreground(fg, now),
            Event::Hook {
                agent,
                signal,
                message,
            } => {
                self.hooks_active = true;
                self.agent = Some(agent);
                let source = Source::Hook(agent);
                let changed = match signal {
                    HookSignal::SessionStart | HookSignal::SessionEnd => {
                        self.set(Status::Idle, source, false, now)
                    }
                    HookSignal::Working => self.set(Status::Running, source, false, now),
                    HookSignal::NeedsInput => self.set(Status::NeedsInput, source, true, now),
                    HookSignal::TurnDone => {
                        if self.focused {
                            self.set(Status::Idle, source, false, now)
                        } else {
                            self.set(Status::Done, source, true, now)
                        }
                    }
                };
                if signal == HookSignal::SessionEnd {
                    self.hooks_active = false;
                }
                if message.is_some() && self.status == Status::NeedsInput {
                    self.message = message;
                    return true;
                }
                changed
            }
            Event::Notify { message } => {
                // An agent whose hooks report questions is better described by them. Codex's
                // hook (`notify`) only reports finished turns, so its approval notifications
                // still count.
                if self.hooks_active && self.agent != Some(AgentKind::Codex) {
                    return false;
                }
                let changed = self.set(Status::NeedsInput, Source::Osc, true, now);
                if message.is_some() {
                    self.message = message;
                    return true;
                }
                changed
            }
            Event::Bell => {
                // Shells ring for a failed completion while the user types; only a bell from a
                // program the user isn't watching asks for attention.
                let program = matches!(self.fg, Foreground::Program { .. });
                if !program || self.focused || self.hooks_active {
                    return false;
                }
                self.set(Status::NeedsInput, Source::Bell, true, now)
            }
            Event::Shell(mark) => {
                self.shell_marks = true;
                match mark {
                    ShellMark::CommandStart => {
                        self.command_started = Some(now);
                        false
                    }
                    ShellMark::CommandFinished(code) => {
                        let Some(start) = self.command_started.take() else {
                            return false;
                        };
                        let long = now.duration_since(start) >= self.long_command;
                        match code {
                            Some(c) if c != 0 && long => {
                                self.set(Status::Failed(c), Source::Shell, true, now)
                            }
                            _ if long => self.set(Status::CommandDone, Source::Shell, true, now),
                            _ => self.set(Status::Idle, Source::Shell, false, now),
                        }
                    }
                }
            }
            Event::Enter => {
                // The user answered the question or sent the agent a prompt.
                let answering = self.status == Status::NeedsInput;
                let prompting = self.hooks_active
                    && self.agent.is_some()
                    && matches!(self.status, Status::Idle | Status::Done);
                if answering || prompting {
                    // With hooks, the agent says when it works; without, nothing would undo it.
                    if self.hooks_active {
                        let back = if answering { Status::Idle } else { self.status };
                        self.provisional = Some((now + ENTER_CONFIRM, back));
                    }
                    self.set(Status::Running, Source::Input, false, now)
                } else {
                    false
                }
            }
            Event::Exited(code) => {
                self.fg = Foreground::Gone;
                self.agent = None;
                self.hooks_active = false;
                match code {
                    Some(c) if c != 0 => self.set(Status::Failed(c), Source::Process, true, now),
                    _ => self.set(Status::Ended, Source::Process, false, now),
                }
            }
        }
    }

    fn foreground(&mut self, fg: Foreground, now: Instant) -> bool {
        if fg == self.fg {
            return false;
        }
        let prev = std::mem::replace(&mut self.fg, fg.clone());
        let ran = now.duration_since(self.fg_since);
        self.fg_since = now;
        match fg {
            Foreground::Shell => match prev {
                Foreground::Program { agent: Some(_), .. } => {
                    // The agent was quit.
                    self.agent = None;
                    self.hooks_active = false;
                    self.set(Status::Idle, Source::Process, false, now)
                }
                Foreground::Program { agent: None, .. } => {
                    // With shell integration the finish mark reports this (with the code).
                    if self.shell_marks {
                        return matches!(self.status, Status::Running)
                            && self.set(Status::Idle, Source::Process, false, now);
                    }
                    if ran >= self.long_command {
                        self.set(Status::CommandDone, Source::Process, true, now)
                    } else {
                        self.set(Status::Idle, Source::Process, false, now)
                    }
                }
                _ => self.set(Status::Idle, Source::Process, false, now),
            },
            Foreground::Program {
                agent: Some(agent), ..
            } => {
                if self.agent != Some(agent) {
                    self.hooks_active = false;
                }
                self.agent = Some(agent);
                if self.hooks_active {
                    return false;
                }
                self.set(Status::Unknown, Source::Process, false, now)
            }
            Foreground::Program { agent: None, .. } => {
                self.set(Status::Running, Source::Process, false, now)
            }
            Foreground::Gone => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(name: &str, agent: Option<AgentKind>) -> Event {
        Event::Foreground(Foreground::Program {
            name: name.into(),
            agent,
        })
    }

    fn hook(signal: HookSignal) -> Event {
        Event::Hook {
            agent: AgentKind::Claude,
            signal,
            message: None,
        }
    }

    #[test]
    fn agent_with_hooks_goes_through_its_turn() {
        let t0 = Instant::now();
        let mut t = Tracker::new(t0);
        t.apply(prog("claude", Some(AgentKind::Claude)), t0);
        assert_eq!(t.status, Status::Unknown, "no hook yet: unknown, not guessed");
        assert_eq!(t.source, Source::Process);
        t.apply(hook(HookSignal::SessionStart), t0);
        assert_eq!(t.status, Status::Idle);
        t.apply(hook(HookSignal::Working), t0);
        assert_eq!(t.status, Status::Running);
        assert!(!t.wants_attention());
        t.apply(
            Event::Hook {
                agent: AgentKind::Claude,
                signal: HookSignal::NeedsInput,
                message: Some("Allow Edit(src/main.rs)?".into()),
            },
            t0,
        );
        assert_eq!(t.status, Status::NeedsInput);
        assert_eq!(t.source, Source::Hook(AgentKind::Claude));
        assert!(t.wants_attention());
        assert_eq!(t.message.as_deref(), Some("Allow Edit(src/main.rs)?"));
        // The user looks: out of the queue, but the question is still open.
        t.set_focused(true, t0);
        assert!(!t.wants_attention());
        assert_eq!(t.status, Status::NeedsInput);
        // Answering (Enter) means the agent works again.
        t.apply(Event::Enter, t0);
        assert_eq!(t.status, Status::Running);
        t.set_focused(false, t0);
        t.apply(hook(HookSignal::TurnDone), t0);
        assert_eq!(t.status, Status::Done);
        assert!(t.wants_attention());
        t.set_focused(true, t0);
        assert_eq!(t.status, Status::Idle, "a seen turn just reads as idle");
        // OSC notifications are ignored while the hooks describe the agent.
        t.set_focused(false, t0);
        assert!(!t.apply(Event::Notify { message: None }, t0));
        // Quitting the agent returns to the shell.
        t.apply(Event::Foreground(Foreground::Shell), t0);
        assert_eq!(t.status, Status::Idle);
        assert_eq!(t.agent, None);
        assert!(!t.hooks_active);
    }

    #[test]
    fn enter_alone_does_not_keep_an_agent_running() {
        let t0 = Instant::now();
        let mut t = Tracker::new(t0);
        t.apply(prog("claude", Some(AgentKind::Claude)), t0);
        t.apply(hook(HookSignal::SessionStart), t0);
        // `/help` or an empty line: Enter, and no hook follows.
        t.apply(Event::Enter, t0);
        assert_eq!(t.status, Status::Running);
        assert_eq!(t.deadline(), Some(t0 + ENTER_CONFIRM));
        assert!(!t.expire(t0 + Duration::from_secs(1)));
        assert!(t.expire(t0 + ENTER_CONFIRM));
        assert_eq!(t.status, Status::Idle);
        // A real prompt: the agent's hook confirms it, and it stays Running.
        t.apply(Event::Enter, t0);
        t.apply(hook(HookSignal::Working), t0);
        assert!(!t.expire(t0 + ENTER_CONFIRM * 2));
        assert_eq!(t.status, Status::Running);
    }

    #[test]
    fn agent_without_hooks_relies_on_osc_and_enter() {
        let t0 = Instant::now();
        let mut t = Tracker::new(t0);
        t.apply(prog("opencode", Some(AgentKind::OpenCode)), t0);
        assert_eq!(t.status, Status::Unknown);
        t.apply(
            Event::Notify {
                message: Some("Permission required".into()),
            },
            t0,
        );
        assert_eq!((t.status, t.source), (Status::NeedsInput, Source::Osc));
        assert!(t.wants_attention());
        t.apply(Event::Enter, t0);
        assert_eq!((t.status, t.source), (Status::Running, Source::Input));
    }

    #[test]
    fn codex_approvals_still_count_after_its_notify_hook() {
        let t0 = Instant::now();
        let mut t = Tracker::new(t0);
        t.apply(prog("codex", Some(AgentKind::Codex)), t0);
        t.apply(
            Event::Hook {
                agent: AgentKind::Codex,
                signal: HookSignal::TurnDone,
                message: None,
            },
            t0,
        );
        assert_eq!(t.status, Status::Done);
        t.apply(Event::Notify { message: None }, t0);
        assert_eq!(t.status, Status::NeedsInput);
    }

    #[test]
    fn long_commands_and_bells() {
        let t0 = Instant::now();
        let mut t = Tracker::new(t0);
        t.apply(prog("cargo", None), t0);
        assert_eq!(t.status, Status::Running);
        // Short: back to idle without a notice.
        t.apply(Event::Foreground(Foreground::Shell), t0 + Duration::from_secs(2));
        assert_eq!(t.status, Status::Idle);
        assert!(!t.wants_attention());
        // Long: "finished" until seen.
        let t1 = t0 + Duration::from_secs(3);
        t.apply(prog("cargo", None), t1);
        t.apply(Event::Foreground(Foreground::Shell), t1 + LONG_COMMAND);
        assert_eq!(t.status, Status::CommandDone);
        assert!(t.wants_attention());
        // A bell from the shell itself is ignored; from a program the user isn't watching, not.
        let mut b = Tracker::new(t0);
        assert!(!b.apply(Event::Bell, t0));
        b.apply(prog("make", None), t0);
        assert!(b.apply(Event::Bell, t0));
        assert_eq!((b.status, b.source), (Status::NeedsInput, Source::Bell));
        let mut c = Tracker::new(t0);
        c.set_focused(true, t0);
        c.apply(prog("make", None), t0);
        assert!(!c.apply(Event::Bell, t0), "the user is watching");
    }

    #[test]
    fn shell_integration_reports_codes_of_long_commands() {
        let t0 = Instant::now();
        let mut t = Tracker::new(t0);
        t.apply(Event::Shell(ShellMark::CommandStart), t0);
        t.apply(prog("cargo", None), t0);
        t.apply(
            Event::Shell(ShellMark::CommandFinished(Some(101))),
            t0 + LONG_COMMAND,
        );
        assert_eq!((t.status, t.source), (Status::Failed(101), Source::Shell));
        // The foreground change that follows does not override the reported failure.
        t.apply(Event::Foreground(Foreground::Shell), t0 + LONG_COMMAND);
        assert_eq!(t.status, Status::Failed(101));
        // A quick failure is not worth a notice.
        let mut q = Tracker::new(t0);
        q.apply(Event::Shell(ShellMark::CommandStart), t0);
        q.apply(Event::Shell(ShellMark::CommandFinished(Some(1))), t0);
        assert_eq!(q.status, Status::Idle);
    }

    #[test]
    fn exit_codes() {
        let t0 = Instant::now();
        let mut t = Tracker::new(t0);
        t.apply(Event::Exited(Some(0)), t0);
        assert_eq!(t.status, Status::Ended);
        assert!(!t.wants_attention());
        let mut f = Tracker::new(t0);
        f.apply(Event::Exited(Some(2)), t0);
        assert_eq!(f.status, Status::Failed(2));
        assert!(f.wants_attention());
    }
}
