// #![deny(elided_lifetimes_in_paths)]
use crossterm::{
    execute, 
    terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    event,
    event::{Event as CEvent, KeyEvent, KeyCode, KeyModifiers},
};
use directories::ProjectDirs;
use mlua::{AsChunk, Lua, Table, Variadic};
use uiua::{Uiua, UiuaResult};

use std::{
    collections::{HashMap, VecDeque}, error::Error, io, mem, sync::mpsc::{self, Sender}, thread, time::{Duration, Instant}
};

use ratatui::{
    backend::CrosstermBackend, layout::Rect, text::{Span, Spans}, widgets::{Block, BorderType, Borders, Paragraph, Wrap}, Terminal
};

struct Calculator {
    stack: Vec<f64>, // TODO: change from f64 to precise value
    text_box: String,
    previous: String,
    operations: HashMap<String, Operation>,
    uiua: Uiua,
    lua: Lua,
    errors: VecDeque<String>,
}

enum Event {
    Input(KeyEvent),
    Submit,
    Tick,
    Quit,
    Reset,
    ClearTextBox,
    PushError(String),
    PopError,
}

enum Operation {
    Rust(Box<dyn Fn(&mut Vec<f64>) -> bool>),
    Uiua(uiua::Function),
    Lua(String, usize),
}

impl Calculator {
    fn new() -> Self {
        Self {
            stack: vec![],
            text_box: "".into(),
            previous: "".into(),
            operations: {
                let mut map = HashMap::new();
                map.insert("+".into(), Operation::new_rust(|&[a, b]| vec![a + b]));
                map.insert("-".into(), Operation::new_rust(|&[a, b]| vec![a - b]));
                map.insert("*".into(), Operation::new_rust(|&[a, b]| vec![a * b]));
                map.insert("/".into(), Operation::new_rust(|&[a, b]| vec![a / b]));
                map.insert("^".into(), Operation::new_rust(|&[a, b]| vec![a.powf(b)]));
                map.insert("neg".into(), Operation::new_rust(|&[a]| vec![-a]));
                map.insert("`".into(), Operation::new_rust(|&[a]| vec![-a]));
                map.insert("sin".into(), Operation::new_rust(|&[a]| vec![a.sin()]));
                map.insert("cos".into(), Operation::new_rust(|&[a]| vec![a.cos()]));
                map.insert("tan".into(), Operation::new_rust(|&[a]| vec![a.tan()]));
                map.insert("asin".into(), Operation::new_rust(|&[a]| vec![a.asin()]));
                map.insert("acos".into(), Operation::new_rust(|&[a]| vec![a.acos()]));
                map.insert("atan".into(), Operation::new_rust(|&[a]| vec![a.atan()]));
                map.insert("d2r".into(), Operation::new_rust(|&[a]| vec![a * std::f64::consts::PI / 180.0]));
                map.insert("ln".into(), Operation::new_rust(|&[a]| vec![a.ln()]));
                map.insert("swap".into(), Operation::new_rust(|&[a, b]| vec![b, a]));
                map.insert("pred".into(), Operation::new_rust(|&[a]| vec![a - 1.]));
                map.insert("succ".into(), Operation::new_rust(|&[a]| vec![a + 1.]));
                map.insert("sqrt".into(), Operation::new_rust(|&[a]| vec![a.sqrt()]));
                map.insert("cbrt".into(), Operation::new_rust(|&[a]| vec![a.cbrt()]));
                map.insert("pi".into(), Operation::new_rust(|&[]| vec![std::f64::consts::PI]));
                map
            },
            uiua: Uiua::with_safe_sys(),
            lua: Lua::new(),
            errors: VecDeque::new(),
        }
    }
    // returns false if unsuccessful. mutates stack and returns true if successful.
    fn operate(&mut self, text: String, tx: Sender<Event>) -> bool {
        self.operations
            .get(&text.to_lowercase())
            .map_or(false, |op| match op {
                Operation::Rust(function) => function(&mut self.stack),
                Operation::Uiua(function) => {
                    let arg_count = function.signature().args;
                    if self.stack.len() >= arg_count {
                        // panic safety: length checked first
                        let (_, stack_top) = self.stack.split_at(self.stack.len() - arg_count);
                        for i in stack_top {
                            self.uiua.push(*i);
                        }
                        let result = self.uiua.call(function.clone());
                        let uiua_stack = self.uiua.take_stack();
                        match result {
                            Ok(()) => {
                                let mut out = Vec::with_capacity(uiua_stack.len());
                                for i in uiua_stack {
                                    match i.as_num(&self.uiua, "") {
                                        Ok(n) => out.push(n),
                                        Err(e) => {
                                            // unwrap safety: rx lasts program lifetime
                                            tx.send(Event::PushError(e.message())).unwrap();
                                            return false;
                                        },
                                    }
                                }
                                for _ in 0..arg_count {self.stack.pop();}
                                self.stack.extend(out);
                                true
                            },
                            Err(e) => {
                                // unwrap safety: rx lasts program lifetime
                                tx.send(Event::PushError(e.message())).unwrap();
                                false
                            }
                        }
                    } else {
                        false
                    }
                },
                Operation::Lua(name, arg_count) => {
                    let table = self.lua.globals().get::<_, Table>("_ripen_registry").unwrap();
                    let function = table.get::<_, mlua::Function>(name.as_str()).unwrap();
                    if self.stack.len() >= *arg_count {
                        // panic safety: length checked first
                        let (_, stack_top) = self.stack.split_at(self.stack.len() - arg_count);
                        let out: mlua::Result<Variadic<f64>> = function.call(Variadic::from_iter(stack_top.iter().copied()));
                        match out {
                            Ok(out) => {
                                for _ in 0..*arg_count {self.stack.pop();}
                                self.stack.extend(out.iter());
                                true
                            },
                            Err(e) => {
                                // unwrap safety: rx lasts program lifetime
                                tx.send(Event::PushError(e.to_string())).unwrap();
                                false
                            }
                        }
                    } else {
                        false
                    }
                },
            })
    }
    fn operate_from_input(&mut self, tx: Sender<Event>) -> bool {
        let text = self.text_box.clone();
        self.operate(text, tx)
    }
    fn operate_previous(&mut self, tx: Sender<Event>) -> bool {
        let text = self.previous.clone();
        self.operate(text, tx)
    }

    fn reset(&mut self) {
        self.stack = Vec::new();
        self.text_box.clear();
        self.previous.clear();
    }

    fn load_lua<'a>(&'a mut self, lua_config: impl AsChunk<'a, 'static>) -> Result<(), mlua::Error> {
        let (name_tx, name_rx) = mpsc::channel();
        self.lua.globals().set("_ripen_registry", self.lua.create_table()?)?;
        let lua_register_function = self.lua.create_function(move |lua, (name, arg_count, func): (String, usize, mlua::Function)| {
            lua.globals().get::<_, Table>("_ripen_registry")?.set(name.clone(), func)?;
            // unwrap safety: rx guaranteed not to have hung up
            name_tx.send((name, arg_count)).unwrap();
            Ok(mlua::Value::Nil)
        })?;
        self.lua.globals().set("register", lua_register_function)?;
        self.lua.load(lua_config).exec()?;
        for (name, arg_count) in name_rx.try_iter() {
            self.operations.insert(name.clone().to_lowercase(), Operation::Lua(name, arg_count));
        }
        Ok(())
    }

    fn load_uiua(&mut self, uiua_config: impl AsRef<std::path::Path>) -> UiuaResult<()> {
        self.uiua.run_file(uiua_config)?;
        for (k, f) in self.uiua.bound_functions() {
            self.operations.insert(k.to_string().to_lowercase(), Operation::Uiua(f));
        }
        Ok(())
    }
}

impl Operation {
    fn new_rust<const N: usize>(op: impl Fn(&[f64; N]) -> Vec<f64> + 'static) -> Self {
        Self::Rust(Box::new(move |v| {
            if v.len() < N {
                return false;
            }
            // unwrap safety: we just checked length
            let (_, nums) = v.split_last_chunk::<N>().unwrap();
            let out = op(nums);
            for _ in 0..N {v.pop();}
            v.extend(out);
            true
        }))
    }
}


fn submit(c: &mut Calculator, tx: Sender<Event>) {
    if let Ok(num) = c.text_box.parse::<f64>() {
        c.stack.push(num);
        c.previous = mem::take(&mut c.text_box);
    } else if c.text_box.is_empty() {
        c.operate_previous(tx);
    } else if c.operate_from_input(tx) {
        c.previous = mem::take(&mut c.text_box);
    }
}

fn main() -> Result<(), Box<dyn Error>>{
    let project_dirs = ProjectDirs::from("", "", "ripen");
    let config_dir = project_dirs.as_ref().map(ProjectDirs::config_local_dir);
    let lua_config = config_dir.map(|p| p.join("functions.lua"));
    let uiua_config = config_dir.map(|p| p.join("functions.ua"));

    let mut app = Calculator::new();
    let (tx, rx) = mpsc::channel();

    // load lua
    if let Some(lua_config) = lua_config {
        if let Err(e) = app.load_lua(lua_config) {
            // unwrap safety: rx lasts program lifetime
            tx.send(Event::PushError(format!("Unable to load Lua config: {e}"))).unwrap();
        }
    } else {
        // unwrap safety: rx lasts program lifetime
        tx.send(Event::PushError("Failed to construct Lua config path".into())).unwrap();
    }
    // TODO: load uiua
    if let Some(uiua_config) = uiua_config {
        if let Err(e) = app.load_uiua(uiua_config) {
            // unwrap safety: rx lasts program lifetime
            tx.send(Event::PushError(format!("Unable to load Uiua config: {e}"))).unwrap();
        }
    } else {
        // unwrap safety: rx lasts program lifetime
        tx.send(Event::PushError("Failed to construct Lua config path".into())).unwrap();
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let keyboard_tx = tx.clone();

    thread::spawn(move || {
        let tx = keyboard_tx;
        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(200);
        loop {
            // Timeout is duration until next tick
            let timeout = tick_rate
                .checked_sub(Instant::now() - last_tick)
                .unwrap_or(Duration::from_secs(0));
            // Wait for events within that duration and send them over the mpsc channel
            // unwrap safety: fatal
            if event::poll(timeout).unwrap() {
            // unwrap safety: fatal
                if let CEvent::Key(key) = event::read().unwrap() {
                    if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        // unwrap safety: rx lasts program lifetime
                        tx.send(Event::Quit).unwrap();
                    } else if key.code == KeyCode::Char('w') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        // unwrap safety: rx lasts program lifetime
                        tx.send(Event::ClearTextBox).unwrap();
                    } else if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        // unwrap safety: rx lasts program lifetime
                        tx.send(Event::Reset).unwrap();
                    } else if key.code == KeyCode::Enter {
                        // unwrap safety: rx lasts program lifetime
                        tx.send(Event::Submit).unwrap();
                    } else {
                        // unwrap safety: rx lasts program lifetime
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
            
            let corner_box = Rect::new(window.width * 2/3, 1, window.width / 3 - 2, stack_size.height - 2);
            let error = Paragraph::new(app.errors.iter().map(Span::raw).map(Spans::from).collect::<Vec<Spans>>()).wrap(Wrap {trim: true});
            f.render_widget(error, corner_box);
        })?;

        // Handle events
        match rx.recv().unwrap() {
            Event::Quit => break,
            Event::Input(KeyEvent {code: KeyCode::Backspace, ..}) => { app.text_box.pop(); },
            Event::Input(KeyEvent {code: KeyCode::Char(chr), ..}) => { app.text_box.push(chr); }
            Event::Submit => { submit(&mut app, tx.clone()); },
            Event::Reset => { app.reset(); },
            Event::ClearTextBox => { mem::take(&mut app.text_box); },
            Event::Tick | Event::Input(..) => {},
            Event::PushError(e) => {
                app.errors.push_back(e);
                let tx = tx.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(4));
                    // unwrap safety: rx lasts program lifetime
                    tx.send(Event::PopError).unwrap();
                });
            },
            Event::PopError => { app.errors.pop_front(); }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
