-- chapters.lua: numbers the HTML the way the printed book is numbered.
--
-- In the LaTeX a \part is the top heading and a \chapter the one below
-- it. The printed book numbers chapters 1, 2, 3 ... straight through,
-- whatever part they are in, and leaves parts out of the numbers. pandoc
-- would number them 1.1, 1.2, 2.1, so this filter numbers them itself.
-- Appendices (after \appendix, marked by a header with the class
-- "appendix-start") are lettered A, B, C.

local chapter, section, lettered = 0, 0, false
local letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"

function Header(h)
  if h.level == 1 then
    h.classes:insert("part")
    h.classes:insert("unnumbered")
    return h
  end
  if h.classes:includes("unnumbered") then
    return h
  end
  local number
  if h.level == 2 then
    chapter = chapter + 1
    section = 0
    if lettered then
      number = letters:sub(chapter, chapter)
    else
      number = tostring(chapter)
    end
    h.classes:insert("chapter")
  elseif h.level == 3 then
    section = section + 1
    local c = lettered and letters:sub(chapter, chapter) or tostring(chapter)
    number = c .. "." .. section
  else
    h.classes:insert("unnumbered")
    return h
  end
  h.classes:insert("unnumbered")
  local label = pandoc.Span({ pandoc.Str(number) }, { class = "number" })
  table.insert(h.content, 1, pandoc.Space())
  table.insert(h.content, 1, label)
  return h
end

function Div(d)
  if d.classes:includes("appendixstart") then
    lettered = true
    chapter = 0
    return {}
  end
end

return { { traverse = "topdown", Div = Div, Header = Header } }
