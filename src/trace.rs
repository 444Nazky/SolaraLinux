use std::sync::Mutex;

static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn log(msg: impl AsRef<str>) {
    let s = msg.as_ref().to_string();
    eprintln!("{s}");
    if let Ok(mut l) = LOG.lock() {
        l.push(s);
        if l.len() > 500 {
            l.drain(..100);
        }
    }
}

pub fn take() -> Vec<String> {
    LOG.lock()
        .map(|mut l| std::mem::take(&mut *l))
        .unwrap_or_default()
}
