#![feature(split_array)]
use crossterm::{
    execute, 
    terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    event,
    event::{Event as CEvent, KeyEvent, KeyCode, KeyModifiers},
};

use std::{
    error::Error, 
    io, 
    thread, 
    sync::mpsc,
    time::{Instant, Duration},
    mem,
};

use ratatui::{
    backend::CrosstermBackend,
    widgets::{Paragraph, Block, Borders, BorderType},
    text::{Span, Spans},
    layout::Rect,
    Terminal,
};

struct Calculator {
    stack: Vec<f64>, // TODO: change from f64 to precise value
    text_box: String,
    previous: String,
    operations: Vec<Operation>,
}

enum Event {
    Input(KeyEvent),
    Submit,
    Tick,
    Quit,
    Reset,
    ClearTextBox
}

struct Operation {
    pred: Box<dyn Fn(&str) -> bool>,
    effect: Box<dyn Fn(&mut Vec<f64>) -> bool>
}

impl Default for Calculator {
    fn default() -> Self {
        Self {
            stack: vec![],
            text_box: "".into(),
            previous: "".into(),
            operations: vec![
                Operation::new(|s| s == "+", |&[a, b]| vec![a + b]),
                Operation::new(|s| s == "-", |&[a, b]| vec![a - b]),
                Operation::new(|s| s == "*", |&[a, b]| vec![a * b]),
                Operation::new(|s| s == "/", |&[a, b]| vec![a / b]),
                Operation::new(|s| s == "^", |&[a, b]| vec![a.powf(b)]),
                Operation::new(|s| s == "inv" || s == "neg", |&[a]| vec![-a]),
                Operation::new(|s| s == "sin", |&[a]| vec![a.sin()]),
                Operation::new(|s| s == "cos", |&[a]| vec![a.cos()]),
                Operation::new(|s| s == "tan", |&[a]| vec![a.tan()]),
                Operation::new(|s| s == "asin", |&[a]| vec![a.asin()]),
                Operation::new(|s| s == "acos", |&[a]| vec![a.acos()]),
                Operation::new(|s| s == "atan", |&[a]| vec![a.atan()]),
                Operation::new(|s| s == "d2r", |&[a]| vec![a * std::f64::consts::PI / 180.0]),
                Operation::new(|s| s == "ln", |&[a]| vec![a.ln()]),
                Operation::new(|s| s == "swap", |&[a, b]| vec![b, a]),
                Operation::new(|s| s == "pred", |&[a]| vec![a - 1.]),
                Operation::new(|s| s == "succ", |&[a]| vec![a + 1.]),
            ],
        }
    }
}

impl Calculator {
    fn operate(&mut self) -> bool {
        let text = self.text_box.as_str();
        self.operations.iter() // All operations
            .find(|op| (op.pred)(text)) // Find one that matches the string
            .map_or(false, |op| (op.effect)(&mut self.stack)) // If it is there, call the
                                                                    // corresponding effect
    }
    fn operate_previous(&mut self) -> bool {
        let text = self.previous.as_str();
        self.operations.iter() // All operations
            .find(|op| (op.pred)(text)) // Find one that matches the string
            .map_or(false, |op| (op.effect)(&mut self.stack)) // If it is there, call the
                                                                    // corresponding effect
    }
}

impl Operation {
    fn new<const N: usize>(pred: impl Fn(&str) -> bool + 'static, op: impl Fn(&[f64; N]) -> Vec<f64> + 'static) -> Self {
        Self { pred: Box::new(pred), effect: Box::new(move |v| {
            if v.len() < N {
                return false;
            }
            let (_, nums) = v.rsplit_array_ref::<N>();
            let out = op(nums);
            for _ in 0..N {v.pop();}
            v.extend(out);
            true

        })}
    }
}


fn submit(c: &mut Calculator) {
    if let Ok(num) = c.text_box.parse::<f64>() {
        c.stack.push(num);
        c.previous = mem::take(&mut c.text_box);
    } else if c.text_box.is_empty() {
        c.operate_previous();
    } else if c.operate() {
        c.previous = mem::take(&mut c.text_box);
    }
}

fn main() -> Result<(), Box<dyn Error>>{
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = Calculator::default();

    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(200);
        loop {
            // Timeout is duration until next tick
            let timeout = tick_rate
                .checked_sub(Instant::now() - last_tick)
                .unwrap_or(Duration::from_secs(0));
            // Wait for events within that duration and send them over the mpsc channel
            if event::poll(timeout).unwrap() {
                if let CEvent::Key(key) = event::read().unwrap() {
                    if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        tx.send(Event::Quit).unwrap();
                    } else if key.code == KeyCode::Char('w') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        tx.send(Event::ClearTextBox).unwrap();
                    } else if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        tx.send(Event::Reset).unwrap();
                    } else if key.code == KeyCode::Enter {
                        tx.send(Event::Submit).unwrap();
                    } else {
                        tx.send(Event::Input(key)).unwrap();
                    }
                }
            }
            // If no inputs received during that time, send a tick event
            if (Instant::now() - last_tick) >= tick_rate && tx.send(Event::Tick).is_ok() {
                last_tick = Instant::now();
            }
        }
    });

    loop {
        // Draw
        terminal.draw(|f| {
            let window = f.size();
            let stack_size = Rect { height: window.height - 3, ..window };
            let stack = Paragraph::new(
                app.stack.iter()
                         .map(|number| Spans::from(Span::raw(format!("{}", number))))
                         .collect::<Vec<Spans>>()
                )
                .scroll(((app.stack.len() as u16).saturating_sub(stack_size.height - 2), 0))
                .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
            let box_size = Rect { height: 3, y: window.height - 3, ..window};
            let text_box = Paragraph::new(Span::from(format!("{}_", app.text_box)))
                .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
            f.render_widget(stack, stack_size);
            f.render_widget(text_box, box_size);
        })?;

        // Handle events
        match rx.recv().unwrap() {
            Event::Quit => break,
            Event::Input(KeyEvent {code: KeyCode::Backspace, ..}) => { app.text_box.pop(); },
            Event::Input(KeyEvent {code: KeyCode::Char(chr), ..}) => { app.text_box.push(chr); }
            Event::Submit => { submit(&mut app); },
            Event::Reset => { mem::take(&mut app); },
            Event::ClearTextBox => { mem::take(&mut app.text_box); },
            Event::Tick | Event::Input(..) => {},
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
