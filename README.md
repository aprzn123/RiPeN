# RiPeN

RiPeN is a simple stack-based (RPN) calculator that runs in a TUI. It supports defining custom operations in both Uiua and Lua.

```uiua
MyOperation ← ⊃+×
```

```lua
-- 2 is the number of values the function reads
register("MyOperation", 2, function(a, b)
    return a * b, a + b
end)
```

Note that operation names are not case-sensitive. Custom operations are defined in `$XDG_CONFIG_HOME/ripen/function.{lua,ua}`. If the same name is defined in both Lua and Uiua, the Uiua function takes priority.
