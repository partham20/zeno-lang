//! The page a Zeno program is in, for the `web` module.
//!
//! There are two. In a browser the page is the real one: every question
//! and every change goes to the browser, through the few lines of
//! JavaScript that started the program (`web_host.rs` and `web.js`).
//!
//! Under `zeno run` there is no browser, so the page is a simulated one,
//! as the board is (`board.rs`). It keeps each element's text, value,
//! attributes and classes, and gives back a line to print for every
//! change that somebody looking at the page would see:
//!
//! ```text
//! #count text: 3 clicks
//! #list + <li> Buy milk
//! #panel hidden
//! ```
//!
//! That is enough to run a program's tests on a desk, and to hold both
//! engines to the same output. The simulated page has no file of HTML
//! behind it, so it has every element a program asks for:
//! `web.element("#count")` makes `#count` the first time it is asked.
//!
//! An element is a number here, as a pin is. On the real page the
//! JavaScript keeps the table that says which element each number is.

use crate::json::Json;
use crate::value::Value;

/// What the simulated page knows of one element.
struct Element {
    /// How it is shown in a line: the selector it was asked for by
    /// (`#count`), or its tag for one the program made (`<li>`).
    name: String,
    /// The tag of an element the program made with `web.create`.
    tag: Option<String>,
    text: String,
    value: String,
    attributes: Vec<(String, String)>,
    classes: Vec<String>,
    hidden: bool,
    /// Is it on the page? One that was asked for is. One that was made
    /// is once it has been put into an element that is.
    on_page: bool,
    parent: Option<u32>,
    children: Vec<u32>,
}

/// The page, real or simulated, and the functions it may call.
pub struct Page {
    real: bool,
    elements: Vec<Element>,
    /// The functions given to `on`, `after` and `on_key`. The page
    /// names one by its place in this list when the time comes.
    handlers: Vec<Value>,
}

/// Ask the real page. Only a program in a browser has one.
#[cfg(target_arch = "wasm32")]
fn ask(request: Vec<Json>) -> Json {
    crate::web_host::ask(&Json::List(request))
}

#[cfg(not(target_arch = "wasm32"))]
fn ask(_: Vec<Json>) -> Json {
    Json::Null
}

fn text(text: &str) -> Json {
    Json::Text(text.to_string())
}

fn number(id: u32) -> Json {
    Json::Number(id as f64)
}

fn as_id(reply: &Json) -> Option<u32> {
    match reply {
        Json::Number(id) => Some(*id as u32),
        _ => None,
    }
}

impl Page {
    /// The page under `zeno run`.
    pub fn simulated() -> Page {
        Page { real: false, elements: Vec::new(), handlers: Vec::new() }
    }

    /// The page in a browser.
    #[cfg(target_arch = "wasm32")]
    pub fn real() -> Page {
        Page { real: true, elements: Vec::new(), handlers: Vec::new() }
    }

    fn at(&self, id: u32) -> &Element {
        &self.elements[id as usize]
    }

    fn at_mut(&mut self, id: u32) -> &mut Element {
        &mut self.elements[id as usize]
    }

    fn add(&mut self, element: Element) -> u32 {
        self.elements.push(element);
        self.elements.len() as u32 - 1
    }

    /// Does this element answer to the selector? The simulated page
    /// reads the plain ones: a tag, `.class` and `#id`, together
    /// (`li.done`) and one inside another (`#list li`).
    fn matches(&self, id: u32, selector: &str) -> bool {
        // An element that was asked for by this selector answers to it.
        if self.at(id).name == selector {
            return true;
        }
        // `#list li.done`: it is an `li.done`, and inside a `#list`.
        let mut steps = selector.split_whitespace().rev();
        if !steps.next().is_some_and(|last| self.is(id, last)) {
            return false;
        }
        let mut around = self.at(id).parent;
        steps.all(|step| loop {
            let Some(outer) = around else { break false };
            around = self.at(outer).parent;
            if self.is(outer, step) {
                break true;
            }
        })
    }

    /// Is this element what one step of a selector says: `li`, `.done`,
    /// `#total`, or several at once, as in `li.done`?
    fn is(&self, id: u32, step: &str) -> bool {
        let element = self.at(id);
        if element.name == step {
            return true;
        }
        // The tag ends where the first `.` or `#` begins a class or an id.
        let tag_ends = step.find(['.', '#']).unwrap_or(step.len());
        let (tag, mut rest) = step.split_at(tag_ends);
        if !tag.is_empty() && element.tag.as_deref() != Some(tag) {
            return false;
        }
        while !rest.is_empty() {
            let ends = rest[1..].find(['.', '#']).map_or(rest.len(), |at| at + 1);
            let (piece, after) = rest.split_at(ends);
            let has = match piece.strip_prefix('.') {
                Some(class) => element.classes.iter().any(|have| have == class),
                None => element.name == piece || element.attributes.iter().any(|(name, id)| name == "id" && piece[1..] == **id),
            };
            if !has {
                return false;
            }
            rest = after;
        }
        true
    }

    /// `web.element("#count")`: the first element that answers to the
    /// selector. `None` when the real page has none. The simulated page
    /// makes one.
    pub fn element(&mut self, selector: &str) -> Option<u32> {
        if self.real {
            return as_id(&ask(vec![text("element"), text(selector)]));
        }
        let found = (0..self.elements.len() as u32).find(|id| self.at(*id).on_page && self.matches(*id, selector));
        Some(found.unwrap_or_else(|| {
            self.add(Element {
                name: selector.to_string(),
                tag: None,
                text: String::new(),
                value: String::new(),
                attributes: Vec::new(),
                classes: Vec::new(),
                hidden: false,
                on_page: true,
                parent: None,
                children: Vec::new(),
            })
        }))
    }

    /// `web.elements("li")`: every element that answers to the selector,
    /// in the order they are on the page. Or why the selector could not
    /// be read.
    pub fn elements(&mut self, selector: &str) -> Result<Vec<u32>, String> {
        if self.real {
            return match ask(vec![text("elements"), text(selector)]) {
                Json::List(ids) => Ok(ids.iter().filter_map(as_id).collect()),
                _ => Err(format!("the browser cannot read `{selector}` as a selector")),
            };
        }
        // A browser reads far more than this page does (`li:first-child`,
        // `a[href]`, `ul > li`). To give an empty list for those would be
        // to say the page has none, which nobody here knows.
        let plain = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '#' | ' ');
        if !selector.chars().all(plain) {
            return Err(format!(
                "the simulated page cannot read `{selector}`. It reads a tag, `.class` and `#id`, together (`li.done`) \
                 and one inside another (`#list li`). A browser reads the rest, so the page that is built can use it"
            ));
        }
        Ok((0..self.elements.len() as u32).filter(|id| self.at(*id).on_page && self.matches(*id, selector)).collect())
    }

    /// `web.create("li", "Buy milk")`: a new element, not yet on the page.
    pub fn create(&mut self, tag: &str, inside: &str) -> u32 {
        if self.real {
            return as_id(&ask(vec![text("create"), text(tag), text(inside)])).unwrap_or(0);
        }
        self.add(Element {
            name: format!("<{tag}>"),
            tag: Some(tag.to_string()),
            text: inside.to_string(),
            value: String::new(),
            attributes: Vec::new(),
            classes: Vec::new(),
            hidden: false,
            on_page: false,
            parent: None,
            children: Vec::new(),
        })
    }

    /// `element.text` or `element.value`.
    pub fn read(&self, id: u32, what: &str) -> String {
        if self.real {
            return match ask(vec![text(what), number(id)]) {
                Json::Text(text) => text,
                _ => String::new(),
            };
        }
        if what == "text" { self.all_text(id) } else { self.at(id).value.clone() }
    }

    /// The text of an element is its own and, after it, the text of
    /// everything in it: a list reads as the text of its items.
    fn all_text(&self, id: u32) -> String {
        let element = self.at(id);
        element.children.iter().fold(element.text.clone(), |text, child| text + &self.all_text(*child))
    }

    /// The line the simulated page prints for a change to an element,
    /// if the element is where somebody could see it.
    fn shows(&self, id: u32, change: String) -> Option<String> {
        let element = self.at(id);
        element.on_page.then(|| format!("{} {change}", element.name))
    }

    /// `element.text = ...` or `element.value = ...`.
    pub fn write(&mut self, id: u32, what: &str, to: &str) -> Option<String> {
        if self.real {
            ask(vec![text(&format!("set_{what}")), number(id), text(to)]);
            return None;
        }
        // What is written again changes nothing that could be seen.
        let element = self.at(id);
        let same = match what {
            "text" => element.text == to && element.children.is_empty(),
            _ => element.value == to,
        };
        if same {
            return None;
        }
        if what == "text" {
            self.at_mut(id).text = to.to_string();
            // New text takes the place of everything that was inside.
            for child in std::mem::take(&mut self.at_mut(id).children) {
                self.leaves(child);
            }
        } else {
            self.at_mut(id).value = to.to_string();
        }
        self.shows(id, format!("{what}: {to}"))
    }

    /// `element.get("href")`: an attribute, or `None` if it has no such.
    pub fn attribute(&self, id: u32, name: &str) -> Option<String> {
        if self.real {
            return match ask(vec![text("get"), number(id), text(name)]) {
                Json::Text(value) => Some(value),
                _ => None,
            };
        }
        self.at(id).attributes.iter().find(|(have, _)| have == name).map(|(_, value)| value.clone())
    }

    /// `element.set("href", "/about")`.
    pub fn set_attribute(&mut self, id: u32, name: &str, value: &str) -> Option<String> {
        if self.real {
            ask(vec![text("set"), number(id), text(name), text(value)]);
            return None;
        }
        let attributes = &mut self.at_mut(id).attributes;
        match attributes.iter_mut().find(|(have, _)| have == name) {
            Some((_, old)) if old == value => return None,
            Some((_, old)) => *old = value.to_string(),
            None => attributes.push((name.to_string(), value.to_string())),
        }
        self.shows(id, format!("{name}: {value}"))
    }

    /// `element.has_class("done")`.
    pub fn has_class(&self, id: u32, class: &str) -> bool {
        if self.real {
            return matches!(ask(vec![text("has_class"), number(id), text(class)]), Json::Bool(true));
        }
        self.at(id).classes.iter().any(|have| have == class)
    }

    /// `element.add_class("done")` and `element.remove_class("done")`.
    /// A class that is already there, or already gone, changes nothing.
    pub fn class(&mut self, id: u32, class: &str, add: bool) -> Option<String> {
        if self.real {
            ask(vec![text(if add { "add_class" } else { "remove_class" }), number(id), text(class)]);
            return None;
        }
        if self.has_class(id, class) == add {
            return None;
        }
        let classes = &mut self.at_mut(id).classes;
        match add {
            true => classes.push(class.to_string()),
            false => classes.retain(|have| have != class),
        }
        self.shows(id, format!("class {}{class}", if add { '+' } else { '-' }))
    }

    /// `element.hide()` and `element.show()`.
    pub fn hide(&mut self, id: u32, hidden: bool) -> Option<String> {
        if self.real {
            ask(vec![text(if hidden { "hide" } else { "show" }), number(id)]);
            return None;
        }
        if self.at(id).hidden == hidden {
            return None;
        }
        self.at_mut(id).hidden = hidden;
        self.shows(id, if hidden { "hidden" } else { "shown" }.to_string())
    }

    /// The element, and everything in it, is on the page or off it.
    fn placed(&mut self, id: u32, on_page: bool) {
        self.at_mut(id).on_page = on_page;
        for child in self.at(id).children.clone() {
            self.placed(child, on_page);
        }
    }

    /// The element is taken out of whatever it is in.
    fn leaves(&mut self, id: u32) {
        if let Some(parent) = self.at_mut(id).parent.take() {
            self.at_mut(parent).children.retain(|child| *child != id);
        }
        self.placed(id, false);
    }

    /// Is `inner` the element `outer`, or somewhere inside it?
    pub fn is_inside(&self, inner: u32, outer: u32) -> bool {
        if self.real {
            return matches!(ask(vec![text("is_inside"), number(inner), number(outer)]), Json::Bool(true));
        }
        let mut at = Some(inner);
        while let Some(id) = at {
            if id == outer {
                return true;
            }
            at = self.at(id).parent;
        }
        false
    }

    /// `parent.append(child)`: the child becomes the last thing in the
    /// parent. An element is in one place, so it leaves where it was.
    pub fn append(&mut self, parent: u32, child: u32) -> Option<String> {
        if self.real {
            ask(vec![text("append"), number(parent), number(child)]);
            return None;
        }
        // Put into something that is not on the page, it leaves the page.
        let left = if self.at(parent).on_page { None } else { self.shows(child, "removed".to_string()) };
        self.leaves(child);
        self.at_mut(child).parent = Some(parent);
        self.at_mut(parent).children.push(child);
        let on_page = self.at(parent).on_page;
        self.placed(child, on_page);
        let (name, inside) = (self.at(child).name.clone(), self.all_text(child));
        let shown = if inside.is_empty() { name } else { format!("{name} {inside}") };
        left.or(self.shows(parent, format!("+ {shown}")))
    }

    /// `element.remove()`: the element leaves the page.
    pub fn remove(&mut self, id: u32) -> Option<String> {
        if self.real {
            ask(vec![text("remove"), number(id)]);
            return None;
        }
        let line = self.shows(id, "removed".to_string());
        self.leaves(id);
        line
    }

    /// `element.clear()`: everything inside the element leaves the page.
    pub fn clear(&mut self, id: u32) -> Option<String> {
        if self.real {
            ask(vec![text("clear"), number(id)]);
            return None;
        }
        let element = self.at_mut(id);
        let was_empty = element.children.is_empty() && element.text.is_empty();
        element.text.clear();
        for child in std::mem::take(&mut self.at_mut(id).children) {
            self.leaves(child);
        }
        if was_empty { None } else { self.shows(id, "cleared".to_string()) }
    }

    /// Keep a function for the page to call later, and say which it is.
    fn keep(&mut self, handler: Value) -> u32 {
        self.handlers.push(handler);
        self.handlers.len() as u32 - 1
    }

    /// The function the page is calling.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn handler(&self, number: u32) -> Option<Value> {
        self.handlers.get(number as usize).cloned()
    }

    /// `element.on("click", pressed)`. The simulated page keeps the
    /// function and never calls it: nobody is there to click.
    pub fn on(&mut self, id: u32, event: &str, handler: Value) {
        let kept = self.keep(handler);
        if self.real {
            ask(vec![text("on"), number(id), text(event), number(kept)]);
        }
    }

    /// `web.after(500 ms, tick)`.
    pub fn after(&mut self, milliseconds: f64, handler: Value) {
        let kept = self.keep(handler);
        if self.real {
            ask(vec![text("after"), Json::Number(milliseconds), number(kept)]);
        }
    }

    /// `web.on_key(pressed)`.
    pub fn on_key(&mut self, handler: Value) {
        let kept = self.keep(handler);
        if self.real {
            ask(vec![text("on_key"), number(kept)]);
        }
    }
}
