use std::sync::Arc;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

/// Manages structured terminal output with spinner-style progress reporting.
#[derive(Clone)]
pub struct Progress {
    multi: Arc<MultiProgress>,
    spinner_style: ProgressStyle,
}

impl Progress {
    /// Constructs a new [`Progress`] manager writing to stderr.
    pub fn new() -> Self {
        let multi = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());
        let spinner_style = ProgressStyle::with_template("{spinner:.green} {msg}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner())
            .tick_strings(&["-", "\\", "|", "/"]);

        Self {
            multi: Arc::new(multi),
            spinner_style,
        }
    }

    /// Starts a new spinner task with the provided label.
    pub fn task(&self, label: impl Into<String>) -> Task {
        let label = label.into();
        let bar = self.multi.add(ProgressBar::new_spinner());
        bar.set_style(self.spinner_style.clone());
        bar.set_message(label.clone());
        bar.enable_steady_tick(Duration::from_millis(80));

        Task {
            bar,
            label,
            finished: false,
        }
    }

    /// Prints a standalone message, respecting the progress draw target.
    pub fn println(&self, message: impl AsRef<str>) {
        let message = message.as_ref();
        // Ensure progress bars are temporarily suspended to avoid interleaving.
        let _ = self.multi.println(message);
    }

    /// Executes a closure while temporarily suspending drawing.
    pub fn suspend<F, T>(&self, operation: F) -> T
    where
        F: FnOnce() -> T,
    {
        self.multi.suspend(operation)
    }
}

/// Spinner-style progress task returned by [`Progress::task`].
pub struct Task {
    bar: ProgressBar,
    label: String,
    finished: bool,
}

impl Task {
    /// Marks the task as successfully completed with a custom trailing message.
    pub fn finish_with_message(mut self, message: impl Into<String>) {
        self.finished = true;
        self.bar.finish_with_message(message.into());
    }

    /// Marks the task as failed, preserving its last message.
    pub fn fail(mut self, message: impl Into<String>) {
        self.finished = true;
        self.bar.abandon_with_message(message.into());
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if !self.finished {
            self.bar
                .abandon_with_message(format!("{} (cancelled)", self.label));
        }
    }
}
