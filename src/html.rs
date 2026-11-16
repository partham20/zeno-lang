//! Reads a page of HTML, for the simulated page.
//!
//! `zeno run todo.zeno` has no browser. If `todo.html` is beside the
//! program, the simulated page (`page.rs`) is given what this file reads
//! out of it: which elements there are, what each is called, its id,
//! its classes and the text in it, and which is inside which. That is
//! what lets `zeno run` say that the page has no `#cuont`, which without
//! it only a browser could (decision 16.9).
//!
//! It is not a browser's reader. A browser mends a page that is written
//! wrong in a hundred ways that are all written down. This reads a
//! page that is written right: a tag that is opened is closed, except
//! the ones that never are (`<br>`, `<input>`) and a `<li>` or the like
//! that the next one closes. What is in `<script>` and `<style>` is
//! skipped.

/// One element of the page as it was written.
#[derive(Debug, Default, PartialEq)]
pub struct Node {
    pub tag: String,
    /// Every attribute as written: `("id", "count")`. One with no value
    /// (`hidden`) has `""`.
    pub attributes: Vec<(String, String)>,
    /// The text written straight inside it, not inside something it holds.
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(have, _)| have == name).map(|(_, value)| value.as_str())
    }
}

/// Tags that have nothing inside them and are never closed.
const EMPTY: &[&str] =
    &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];

/// A tag that is closed by the next one of its kind: `<li>one<li>two`.
/// Each is named with what it must be inside for that to hold.
const CLOSED_BY_NEXT: &[(&str, &[&str])] = &[
    ("li", &["ul", "ol"]),
    ("option", &["select", "datalist"]),
    ("tr", &["table", "tbody", "thead", "tfoot"]),
    ("td", &["tr"]),
    ("th", &["tr"]),
    ("dt", &["dl"]),
    ("dd", &["dl"]),
];

/// `&amp;` and its like, as the characters they stand for.
fn plain(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out += &rest[..at];
        rest = &rest[at..];
        let known = [("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>'), ("&quot;", '"'), ("&#39;", '\''), ("&apos;", '\''), ("&nbsp;", '\u{a0}')];
        match known.iter().find(|(name, _)| rest.starts_with(name)) {
            Some((name, c)) => {
                out.push(*c);
                rest = &rest[name.len()..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out + rest
}

/// The elements of a page, outermost first. The answer is one node with
/// no tag that holds whatever the page has at its top.
pub fn read(html: &str) -> Node {
    // The elements that are open, innermost last. The first is the page.
    let mut open: Vec<Node> = vec![Node::default()];
    let mut rest = html;

    // The innermost open element is finished: it goes into the one around it.
    fn close(open: &mut Vec<Node>) {
        if open.len() > 1 {
            let done = open.pop().expect("more than one");
            open.last_mut().expect("the page").children.push(done);
        }
    }

    while !rest.is_empty() {
        let Some(at) = rest.find('<') else {
            open.last_mut().expect("the page").text += &plain(rest);
            break;
        };
        open.last_mut().expect("the page").text += &plain(&rest[..at]);
        rest = &rest[at..];

        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            rest = rest.find('>').map_or("", |end| &rest[end + 1..]);
            continue;
        }
        if let Some(after) = rest.strip_prefix("</") {
            let end = after.find('>').unwrap_or(after.len());
            let tag = after[..end].trim().to_ascii_lowercase();
            rest = after.get(end + 1..).unwrap_or("");
            // Close up to the one this closes. A closing tag for
            // nothing that is open is left out.
            if let Some(depth) = open.iter().rposition(|node| node.tag == tag) {
                if depth > 0 {
                    while open.len() > depth {
                        close(&mut open);
                    }
                }
            }
            continue;
        }
        // An opening tag, if a name follows. A `<` that is only text
        // (`a < b`) stays text.
        let name_ends = rest[1..].find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).map_or(rest.len(), |n| n + 1);
        if name_ends == 1 || !rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            open.last_mut().expect("the page").text.push('<');
            rest = &rest[1..];
            continue;
        }
        let tag = rest[1..name_ends].to_ascii_lowercase();
        rest = &rest[name_ends..];

        // Its attributes, up to the `>`.
        let mut attributes = Vec::new();
        let mut closed_itself = false;
        loop {
            rest = rest.trim_start();
            if let Some(after) = rest.strip_prefix("/>") {
                closed_itself = true;
                rest = after;
                break;
            }
            if let Some(after) = rest.strip_prefix('>') {
                rest = after;
                break;
            }
            if rest.is_empty() {
                break;
            }
            let ends = rest.find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/')).unwrap_or(rest.len());
            if ends == 0 {
                // A stray `/` or `=`.
                rest = &rest[1..];
                continue;
            }
            let name = rest[..ends].to_ascii_lowercase();
            rest = rest[ends..].trim_start();
            let mut value = String::new();
            if let Some(after) = rest.strip_prefix('=') {
                let after = after.trim_start();
                match after.chars().next() {
                    Some(quote @ ('"' | '\'')) => {
                        let inside = &after[1..];
                        let end = inside.find(quote).unwrap_or(inside.len());
                        value = plain(&inside[..end]);
                        rest = inside.get(end + 1..).unwrap_or("");
                    }
                    _ => {
                        let end = after.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(after.len());
                        value = plain(&after[..end]);
                        rest = &after[end..];
                    }
                }
            }
            attributes.push((name, value));
        }

        // `<li>` closes the `<li>` before it, if no list was opened since.
        if let Some((_, inside)) = CLOSED_BY_NEXT.iter().find(|(closed, _)| *closed == tag) {
            let nearest = open.iter().rposition(|node| node.tag == tag || inside.contains(&node.tag.as_str()));
            if let Some(depth) = nearest.filter(|depth| open[*depth].tag == tag) {
                while open.len() > depth {
                    close(&mut open);
                }
            }
        }

        let node = Node { tag: tag.clone(), attributes, text: String::new(), children: Vec::new() };
        if closed_itself || EMPTY.contains(&tag.as_str()) {
            open.last_mut().expect("the page").children.push(node);
            continue;
        }
        open.push(node);
        // What a script or a style holds is not the page's text, and may
        // have a `<` in it that opens nothing.
        if tag == "script" || tag == "style" {
            let closing = format!("</{tag}");
            let end = rest.to_ascii_lowercase().find(&closing).unwrap_or(rest.len());
            rest = &rest[end..];
        }
    }
    while open.len() > 1 {
        close(&mut open);
    }
    open.pop().expect("the page")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page as nested text: `ul#list.big(li"one" li"two")`.
    fn shape(node: &Node) -> String {
        let mut out = node.tag.clone();
        if let Some(id) = node.attribute("id") {
            out += &format!("#{id}");
        }
        if let Some(class) = node.attribute("class") {
            out += &format!(".{}", class.replace(' ', "."));
        }
        if !node.text.trim().is_empty() {
            out += &format!("{:?}", node.text.trim());
        }
        if !node.children.is_empty() {
            out += &format!("({})", node.children.iter().map(shape).collect::<Vec<_>>().join(" "));
        }
        out
    }

    #[test]
    fn reads_a_page() {
        let page = read(
            "<!doctype html>\n<html><head><title>T</title><style>li < p { }</style></head>\n\
             <body><!-- <p id=no> --><form id=\"add\"><input id='what' placeholder=\"a > b\" autofocus><button>Add</button></form>\n\
             <ul id=list class=\"big wide\"><li>one<li class=done>two &amp; three</ul><p id=left>1 &lt; 2<br/>x</p>\n\
             <script>if (a < b) { document.write('<p id=\"fake\">') }</script></body></html>",
        );
        assert_eq!(
            shape(&page),
            "(html(head(title\"T\" style) body(form#add(input#what button\"Add\") \
             ul#list.big.wide(li\"one\" li.done\"two & three\") p#left\"1 < 2x\"(br) script)))"
        );
        let input = &page.children[0].children[1].children[0].children[0];
        assert_eq!(input.attribute("placeholder"), Some("a > b"));
        assert_eq!(input.attribute("autofocus"), Some(""));
    }

    #[test]
    fn a_page_written_wrong_is_still_read() {
        assert_eq!(shape(&read("<div><p>one</div></span><b>two")), "(div(p\"one\") b\"two\")");
        assert_eq!(shape(&read("a < b <")), "\"a < b <\"");
        assert_eq!(shape(&read("<ul><li>a<ul><li>b</ul><li>c</ul>")), "(ul(li\"a\"(ul(li\"b\")) li\"c\"))");
    }
}
