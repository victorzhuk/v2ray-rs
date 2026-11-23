use super::*;

impl ProcessManager {
    pub(super) fn push_notice(&self, content: String) {
        write_stream_line(&self.log_writer, "notice", &content);
        let line = LogLine::stderr(content);
        self.log_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(line.clone());
        self.state.emit(ProcessEvent::LogLine(line));
    }
    pub(super) fn capture_output(&mut self, child: &mut Child) {
        if let Some(stdout) = child.stdout.take() {
            let tx = self.state.log_sender().clone();
            let buffer = Arc::clone(&self.log_buffer);
            let writer = self.log_writer.clone();
            self.log_handles.push(tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                let mut buf = Vec::new();
                while let Ok(Some(line)) =
                    read_bounded_line(&mut reader, &mut buf, MAX_LOG_LINE_BYTES).await
                {
                    let line = strip_ansi(&line);
                    write_stream_line(&writer, "stdout", &line);
                    let log_line = LogLine::stdout(line);
                    let _ = tx.send(ProcessEvent::LogLine(log_line.clone()));
                    buffer
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(log_line);
                }
            }));
        }

        if let Some(stderr) = child.stderr.take() {
            let tx = self.state.log_sender().clone();
            let buffer = Arc::clone(&self.log_buffer);
            let writer = self.log_writer.clone();
            self.log_handles.push(tokio::spawn(async move {
                let mut reader = BufReader::new(stderr);
                let mut buf = Vec::new();
                while let Ok(Some(line)) =
                    read_bounded_line(&mut reader, &mut buf, MAX_LOG_LINE_BYTES).await
                {
                    let line = strip_ansi(&line);
                    write_stream_line(&writer, "stderr", &line);
                    let log_line = LogLine::stderr(line);
                    let _ = tx.send(ProcessEvent::LogLine(log_line.clone()));
                    buffer
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(log_line);
                }
            }));
        }
    }

    pub(super) async fn graceful_stop(&mut self) {
        self.stop_child(true, self.stop_reason.as_str()).await;
    }
}
