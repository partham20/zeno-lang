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
//! behind it unless `zeno run` found one beside the program
//! (`todo.html` for `todo.zeno`, read by `html.rs`). With one, it has
//! the elements that file has, and asking for another is the error it
//! is in a browser. With none, it has every element a program asks for:
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
    /// What `set_style` gave it: `("color", "red")`.
    styles: Vec<(String, String)>,
    hidden: bool,
    /// Is it on the page? One that was asked for is. One that was made
    /// is once it has been put into an element that is.
    on_page: bool,
    parent: Option<u32>,
    children: Vec<u32>,
}

impl Element {
    /// An element with this name and nothing else.
    fn named(name: String, tag: Option<String>, on_page: bool) -> Element {
        Element {
            name,
            tag,
            text: String::new(),
            value: String::new(),
            attributes: Vec::new(),
            classes: Vec::new(),
            styles: Vec::new(),
            hidden: false,
            on_page,
            parent: None,
            children: Vec::new(),
        }
    }
}

/// A function the page was given to call, and whether it asked for the
/// `Event`: `function(event: Event) { ... }` does, `function() { ... }`
/// does not.
#[derive(Clone)]
pub struct Handler {
    pub function: Value,
    pub takes_event: bool,
}

/// The page, real or simulated, and the functions it may call.
pub struct Page {
    real: bool,
    /// The simulated page was read from a file of HTML, so it has the
    /// elements that file has and no others.
    written: bool,
    elements: Vec<Element>,
    /// The functions given to `on`, `after`, `every`, `on_key`, `fetch`
    /// and `post`. The page names one by its place in this list when
    /// the time comes.
    handlers: Vec<Handler>,
    /// Which function was given to `on` for which event of which
    /// element, in the order given. `fire` calls them.
    listeners: Vec<(u32, String, u32)>,
    /// What `web.save` kept, on the simulated page.
    saved: Vec<(String, String)>,
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
    fn new(real: bool) -> Page {
        Page { real, written: false, elements: Vec::new(), handlers: Vec::new(), listeners: Vec::new(), saved: Vec::new() }
    }

    /// The page under `zeno run`. `html` is the page that was written
    /// for the program, if there is one beside it.
    pub fn simulated(html: Option<&str>) -> Page {
        let mut page = Page::new(false);
        if let Some(html) = html {
            page.written = true;
            for node in &crate::html::read(html).children {
                page.read_in(node, None);
            }
        }
        page
    }

    /// The page in a browser.
    #[cfg(target_arch = "wasm32")]
    pub fn real() -> Page {
        Page::new(true)
    }

    /// Take an element of the written page, and all it holds, onto the
    /// simulated one.
    fn read_in(&mut self, node: &crate::html::Node, parent: Option<u32>) {
        let name = match node.attribute("id") {
            Some(id) => format!("#{id}"),
            None => format!("<{}>", node.tag),
        };
        let mut element = Element::named(name, Some(node.tag.clone()), true);
        // Text that is only the spaces a page is laid out with is none.
        if !node.text.trim().is_empty() {
            element.text = node.text.clone();
        }
        element.classes = node.attribute("class").unwrap_or("").split_whitespace().map(str::to_string).collect();
        element.hidden = node.attribute("hidden").is_some();
        // What a box holds when the page opens.
        element.value = match node.tag.as_str() {
            "textarea" => std::mem::take(&mut element.text),
            _ => node.attribute("value").unwrap_or("").to_string(),
        };
        element.attributes = node.attributes.clone();
        element.parent = parent;
        let id = self.add(element);
        if let Some(parent) = parent {
            self.at_mut(parent).children.push(id);
        }
        for child in &node.children {
            self.read_in(child, Some(id));
        }
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
    /// selector. `None` when the page has none. A simulated page that
    /// was not read from a file makes one.
    pub fn element(&mut self, selector: &str) -> Option<u32> {
        if self.real {
            return as_id(&ask(vec![text("element"), text(selector)]));
        }
        let found = (0..self.elements.len() as u32).find(|id| self.at(*id).on_page && self.matches(*id, selector));
        if self.written {
            return found;
        }
        Some(found.unwrap_or_else(|| self.add(Element::named(selector.to_string(), None, true))))
    }

    /// Can the simulated page read this selector? It reads a tag,
    /// `.class` and `#id`, together and one inside another.
    pub fn reads(&self, selector: &str) -> bool {
        let plain = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '#' | ' ');
        self.real || selector.chars().all(plain)
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
        if !self.reads(selector) {
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
        let mut element = Element::named(format!("<{tag}>"), Some(tag.to_string()), false);
        element.text = inside.to_string();
        self.add(element)
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

    /// How the element is shown in a line: `#count`, or `<li>`.
    pub fn name(&self, id: u32) -> String {
        if self.real {
            return match ask(vec![text("name"), number(id)]) {
                Json::Text(name) => name,
                _ => String::new(),
            };
        }
        self.at(id).name.clone()
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

    /// `element.get_style("color")`: what `set_style` gave it, or what
    /// the page wrote in its `style`. `None` if neither did.
    pub fn style(&self, id: u32, name: &str) -> Option<String> {
        if self.real {
            return match ask(vec![text("get_style"), number(id), text(name)]) {
                Json::Text(value) => Some(value),
                _ => None,
            };
        }
        let element = self.at(id);
        if let Some((_, value)) = element.styles.iter().find(|(have, _)| have == name) {
            return Some(value.clone());
        }
        // `style="color: red; width: 10px"` as the page wrote it.
        let written = element.attributes.iter().find(|(have, _)| have == "style").map(|(_, style)| style.as_str())?;
        written.split(';').filter_map(|one| one.split_once(':')).find(|(have, _)| have.trim() == name).map(|(_, value)| value.trim().to_string())
    }

    /// `element.set_style("color", "red")`. `Err` if the browser has no
    /// style of that name.
    pub fn set_style(&mut self, id: u32, name: &str, value: &str) -> Result<Option<String>, ()> {
        if self.real {
            return match ask(vec![text("set_style"), number(id), text(name), text(value)]) {
                Json::Bool(false) => Err(()),
                _ => Ok(None),
            };
        }
        if self.style(id, name).as_deref() == Some(value) {
            return Ok(None);
        }
        let styles = &mut self.at_mut(id).styles;
        match styles.iter_mut().find(|(have, _)| have == name) {
            Some((_, old)) => *old = value.to_string(),
            None => styles.push((name.to_string(), value.to_string())),
        }
        Ok(self.shows(id, format!("style {name}: {value}")))
    }

    /// `board.draw_rect(10, 10, 50, 20, "red")` and the other four: draw
    /// on a `<canvas>`. `what` is `rect`, `circle`, `line`, `text` or
    /// `erase`, `numbers` the places and sizes, `words` the text and the
    /// colour. `Err` if the element is not a canvas.
    pub fn draw(&mut self, id: u32, what: &str, numbers: &[f64], words: &[&str]) -> Result<Option<String>, ()> {
        if self.real {
            let numbers = Json::List(numbers.iter().map(|n| Json::Number(*n)).collect());
            let words = Json::List(words.iter().map(|word| text(word)).collect());
            return match ask(vec![text("draw"), number(id), text(what), numbers, words]) {
                Json::Bool(false) => Err(()),
                _ => Ok(None),
            };
        }
        // An element the simulated page made up may be anything. One
        // whose tag it knows must be a canvas.
        if self.at(id).tag.as_deref().is_some_and(|tag| tag != "canvas") {
            return Err(());
        }
        let mut line = if what == "erase" { what.to_string() } else { format!("draw_{what}") };
        for n in numbers {
            // 10 and not 10.0: a place on a canvas is mostly a whole number.
            line += &if n.fract() == 0.0 { format!(" {}", *n as i64) } else { format!(" {n}") };
        }
        for word in words {
            line += &format!(" {word}");
        }
        Ok(self.shows(id, line))
    }

    /// `web.save("name", "Partha")`: keep text under a name, for the
    /// next visit to the page.
    pub fn save(&mut self, name: &str, value: &str) {
        if self.real {
            ask(vec![text("save"), text(name), text(value)]);
            return;
        }
        match self.saved.iter_mut().find(|(have, _)| have == name) {
            Some((_, old)) => *old = value.to_string(),
            None => self.saved.push((name.to_string(), value.to_string())),
        }
    }

    /// `web.load("name")`: what was saved under the name, if anything was.
    pub fn load(&self, name: &str) -> Option<String> {
        if self.real {
            return match ask(vec![text("load"), text(name)]) {
                Json::Text(value) => Some(value),
                _ => None,
            };
        }
        self.saved.iter().find(|(have, _)| have == name).map(|(_, value)| value.clone())
    }

    /// `web.forget("name")`: nothing is saved under the name any more.
    pub fn forget(&mut self, name: &str) {
        if self.real {
            ask(vec![text("forget"), text(name)]);
            return;
        }
        self.saved.retain(|(have, _)| have != name);
    }

    /// Keep a function for the page to call later, and say which it is.
    fn keep(&mut self, function: Value, takes_event: bool) -> u32 {
        self.handlers.push(Handler { function, takes_event });
        self.handlers.len() as u32 - 1
    }

    /// The function the page is calling.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn handler(&self, number: u32) -> Option<Handler> {
        self.handlers.get(number as usize).cloned()
    }

    /// `element.on("click", pressed)`. The simulated page keeps the
    /// function and calls it only when the program says `fire`: nobody
    /// is there to click.
    pub fn on(&mut self, id: u32, event: &str, handler: Value, takes_event: bool) {
        let kept = self.keep(handler, takes_event);
        self.listeners.push((id, event.to_string(), kept));
        if self.real {
            ask(vec![text("on"), number(id), text(event), number(kept)]);
        }
    }

    /// The functions that `on` was given for this event of this
    /// element, in the order given: what `fire` calls.
    pub fn listeners(&self, id: u32, event: &str) -> Vec<Handler> {
        let heard = self.listeners.iter().filter(|(element, kind, _)| *element == id && kind == event);
        heard.map(|(_, _, kept)| self.handlers[*kept as usize].clone()).collect()
    }

    /// `web.after(500 ms, tick)`, and `web.every(1 s, tick)` with `again`.
    /// What comes back is the number the timer is known by.
    pub fn after(&mut self, milliseconds: f64, handler: Value, again: bool) -> u32 {
        let kept = self.keep(handler, false);
        if self.real {
            ask(vec![text(if again { "every" } else { "after" }), Json::Number(milliseconds), number(kept)]);
        }
        kept
    }

    /// `timer.stop()`: the page is told not to call any more. The
    /// simulated page calls nothing, so it has nothing to call off.
    pub fn stop(&mut self, timer: u32) {
        if self.real {
            ask(vec![text("stop"), number(timer)]);
        }
    }

    /// `web.on_key(pressed)`.
    pub fn on_key(&mut self, handler: Value) {
        let kept = self.keep(handler, false);
        if self.real {
            ask(vec![text("on_key"), number(kept)]);
        }
    }

    /// `web.fetch(address, done)` and `web.post(address, body, done)`:
    /// ask the network, and call `done` with the answer when it comes.
    /// The simulated page has no network and calls nothing.
    pub fn fetch(&mut self, address: &str, body: Option<&str>, handler: Value) {
        let kept = self.keep(handler, false);
        if self.real {
            let body = body.map_or(Json::Null, text);
            ask(vec![text("fetch"), text(address), body, number(kept)]);
        }
    }
}
