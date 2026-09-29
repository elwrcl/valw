use std::fmt;
use std::path::Path;

/// A concrete next step for the user, attached to an error with [`HintExt::hint`].
#[derive(Debug)]
pub struct Hint(pub String);

impl fmt::Display for Hint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The user cancelled the capture. Not a failure; maps to exit code 3.
#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub trait HintExt<T> {
    fn hint(self, hint: impl Into<String>) -> anyhow::Result<T>;
}

impl<T, E> HintExt<T> for Result<T, E>
where
    E: Into<anyhow::Error>,
{
    fn hint(self, hint: impl Into<String>) -> anyhow::Result<T> {
        self.map_err(|e| e.into().context(Hint(hint.into())))
    }
}

/// Renders an error as `error:` / `cause:` / `hint:` / `log:` lines.
pub fn render(err: &anyhow::Error, log: Option<&Path>) -> String {
    let hint = err.downcast_ref::<Hint>().map(|h| h.0.as_str());
    // Hints live in the context chain too; skip them so they print once.
    let mut messages = err
        .chain()
        .map(|e| e.to_string())
        .filter(|m| Some(m.as_str()) != hint);

    let mut out = format!("error: {}\n", messages.next().unwrap_or_default());
    for cause in messages {
        out.push_str(&format!("  cause: {cause}\n"));
    }
    if let Some(hint) = hint {
        out.push_str(&format!("  hint:  {hint}\n"));
    }
    if let Some(log) = log {
        out.push_str(&format!("  log:   {}\n", log.display()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Context, anyhow};
    use std::path::PathBuf;

    #[test]
    fn renders_all_parts() {
        let err = Err::<(), _>(anyhow!("compositor offered only XBGR2101010"))
            .context("could not convert the frame")
            .hint("run `valw doctor` and include its output in a bug report")
            .context("could not capture output HDMI-A-1")
            .unwrap_err();
        let log = PathBuf::from("/tmp/valw.log");

        assert_eq!(
            render(&err, Some(&log)),
            "error: could not capture output HDMI-A-1\n\
             \x20 cause: could not convert the frame\n\
             \x20 cause: compositor offered only XBGR2101010\n\
             \x20 hint:  run `valw doctor` and include its output in a bug report\n\
             \x20 log:   /tmp/valw.log\n"
        );
    }

    #[test]
    fn renders_without_hint_or_log() {
        let err = anyhow!("disk full");
        assert_eq!(render(&err, None), "error: disk full\n");
    }

    #[test]
    fn hint_as_outermost_layer_is_not_the_headline() {
        let err = Err::<(), _>(anyhow!("valw is already running"))
            .hint("wait for the other capture to finish")
            .unwrap_err();
        assert_eq!(
            render(&err, None),
            "error: valw is already running\n\
             \x20 hint:  wait for the other capture to finish\n"
        );
    }

    #[test]
    fn cancelled_is_detectable() {
        let err = anyhow::Error::new(Cancelled).context("region selection");
        assert!(err.downcast_ref::<Cancelled>().is_some());
    }
}
