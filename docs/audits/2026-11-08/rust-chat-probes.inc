#[cfg(test)]
mod audit_20261108_chat {
    #[test]
    fn legitimate_shared_html_boilerplate_triggers_early_stream_stop() {
        let mut shared = String::from("<header><h1>Project dashboard</h1></header>\n<table>\n");
        for n in 0..12 {
            shared.push_str(&format!("<tr><td>Required field {n}</td><td>Enter the value for field {n}</td><td>Pending</td></tr>\n"));
        }
        let text = format!("Here are the requested two standalone templates with the same header and fields.\n## First page\n```html\n{shared}</table>\n```\n## Second page\n```html\n{shared}");
        let cut = super::repetition_cut(&text).expect("the shared boilerplate in the second legitimate code block triggers the detector");
        assert!(cut < text.len());
        assert!(text.len() - cut >= 400);
        println!("CONFIRMED: valid shared HTML boilerplate is cut from {} bytes to {cut}; caller turns this into finish_reason=stop", text.len());
    }
}
