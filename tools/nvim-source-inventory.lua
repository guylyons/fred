-- nvim-source-inventory.lua: Neovim core surface → docs/nvim-parity.csv.
--
-- Run with a clean Neovim, never the user's config:
--   nvim --clean -l tools/nvim-source-inventory.lua SOURCE docs/nvim-parity.csv
-- SOURCE is the pinned checkout (see docs/nvim-core-roadmap.md). Every ex
-- command (src/nvim/ex_cmds.lua), option (src/nvim/options.lua), key of
-- every mode (runtime/doc/index.txt) and autocommand event
-- (src/nvim/auevents.lua) becomes a row. Re-running keeps the state and
-- fred_behavior already recorded in OUTPUT; new rows start from SCOPE below.

local source, output = arg[1], arg[2]
assert(source and output, 'usage: nvim -l nvim-source-inventory.lua SOURCE OUTPUT.csv')

local function set(list)
  local s = {}
  for _, v in ipairs(list) do
    s[v] = true
  end
  return s
end

-- What is out of scope (excluded) or not blocking (deferred), with why.
local SCOPE = {
  ex = {
    {
      'excluded',
      'Vimscript language',
      set({
        'let', 'unlet', 'const', 'lockvar', 'unlockvar', 'if', 'else', 'elseif',
        'endif', 'for', 'endfor', 'while', 'endwhile', 'break', 'continue', 'try',
        'catch', 'finally', 'endtry', 'throw', 'function', 'endfunction',
        'delfunction', 'return', 'call', 'defer', 'eval', 'echo', 'echoerr',
        'echohl', 'echomsg', 'echon', 'execute', 'finish', 'source', 'runtime',
        'scriptnames', 'scriptencoding', 'debug', 'debuggreedy', 'breakadd',
        'breakdel', 'breaklist', 'profile', 'profdel', 'syntime', 'sandbox', 'redir',
      }),
    },
    {
      'excluded',
      'other languages, plugins and remote UI',
      set({
        'lua', 'luado', 'luafile', 'perl', 'perldo', 'perlfile', 'python', 'pydo',
        'pyfile', 'py3', 'py3do', 'python3', 'py3file', 'pyx', 'pyxdo', 'pythonx',
        'pyxfile', 'ruby', 'rubydo', 'rubyfile', 'tcl', 'tcldo', 'tclfile',
        'mzscheme', 'mzfile', 'packadd', 'packdel', 'packloadall', 'packupdate',
        'checkhealth', 'lsp', 'trust', 'connect', 'detach', 'restart', 'log',
      }),
    },
    {
      'excluded',
      'GUI',
      set({ 'gui', 'gvim', 'simalt', 'winpos', 'browse', 'popup' }),
    },
    { 'deferred', 'diff mode', set({ 'diffupdate', 'diffget', 'diffoff', 'diffpatch', 'diffput', 'diffsplit', 'diffthis' }) },
    {
      'deferred',
      'spell checking',
      set({ 'spellgood', 'spelldump', 'spellinfo', 'spellrepall', 'spellrare', 'spellundo', 'spellwrong', 'mkspell' }),
    },
    {
      'deferred',
      'sessions, views and shada',
      set({ 'mksession', 'mkview', 'loadview', 'mkexrc', 'mkvimrc', 'rshada', 'wshada', 'rviminfo', 'wviminfo' }),
    },
    { 'deferred', 'terminal', set({ 'terminal' }) },
    { 'deferred', 'help system', set({ 'help', 'helpclose', 'helptags', 'helpgrep', 'lhelpgrep' }) },
  },
  option = {
    {
      'excluded',
      'GUI',
      set({
        'guifont', 'guifontwide', 'guioptions', 'guitablabel', 'guitabtooltip',
        'linespace', 'browsedir', 'winaltkeys', 'mousehide', 'mousefocus',
        'menuitems', 'icon', 'iconstring',
      }),
    },
    {
      'excluded',
      'Vimscript, plugins and other languages',
      set({
        'loadplugins', 'packpath', 'packlockfile', 'runtimepath', 'pyxversion',
        'maxfuncdepth', 'modelineexpr', 'secure', 'exrc', 'channel', 'busy',
      }),
    },
    { 'deferred', 'diff mode', set({ 'diff', 'diffanchors', 'diffexpr', 'diffopt', 'patchexpr' }) },
    {
      'deferred',
      'spell checking',
      set({ 'spell', 'spellcapcheck', 'spellfile', 'spelllang', 'spelloptions', 'spellsuggest', 'mkspellmem' }),
    },
    {
      'deferred',
      'sessions, views and shada',
      set({ 'sessionoptions', 'viewdir', 'viewoptions', 'shada', 'shadafile' }),
    },
    { 'deferred', 'terminal', set({ 'scrollback', 'termpastefilter', 'termsync' }) },
    { 'deferred', 'help system', set({ 'helpfile', 'helpheight', 'helplang' }) },
    {
      'deferred',
      'right-to-left text and input methods',
      set({
        'aleph', 'allowrevins', 'arabic', 'arabicshape', 'delcombine', 'hkmap',
        'hkmapp', 'imcmdline', 'imdisable', 'iminsert', 'imsearch', 'keymap',
        'langmap', 'langmenu', 'langnoremap', 'langremap', 'revins', 'rightleft',
        'rightleftcmd', 'termbidi',
      }),
    },
  },
  key = {},
  event = {
    {
      'excluded',
      'Vimscript, Lua, plugins and remote UI',
      set({
        'ChanInfo', 'ChanOpen', 'FuncUndefined', 'SourceCmd', 'SourcePost',
        'SourcePre', 'SessionLoadPre', 'LspAttach', 'LspDetach', 'LspNotify',
        'LspProgress', 'LspRequest', 'LspTokenUpdate', 'PackChangedPre',
        'PackChanged', 'UIEnter', 'UILeave', 'Signal', 'Progress', 'User',
      }),
    },
  },
}

-- Option types that hold a Vimscript expression or function: deferred.
local EXPR_TYPES = set({ 'expr', 'func' })
-- index.txt section → the mode its keys belong to.
local KEY_MODES = {
  ['1'] = 'insert',
  ['2'] = 'normal',
  ['2.1'] = 'textobject',
  ['2.2'] = 'ctrl-w',
  ['2.3'] = 'bracket',
  ['2.4'] = 'g',
  ['2.5'] = 'z',
  ['2.6'] = 'operator-pending',
  ['3'] = 'visual',
  ['4'] = 'cmdline',
  ['5'] = 'terminal',
}

local function read(path)
  local f = assert(io.open(source .. '/' .. path))
  local s = f:read('a')
  f:close()
  return s
end

local function scope(kind, name, mode)
  if kind == 'key' and mode == 'terminal' then
    return 'deferred', 'terminal'
  end
  if kind == 'ex' and name:find('menu', 1, true) then
    return 'excluded', 'GUI'
  end
  for _, rule in ipairs(SCOPE[kind] or {}) do
    if rule[3][name] then
      return rule[1], rule[2]
    end
  end
  return 'missing', ''
end

local rows = {}
local function add(kind, mode, symbol, detail, doc)
  local state, why = scope(kind, symbol, mode)
  rows[#rows + 1] = {
    kind = kind,
    mode = mode,
    symbol = symbol,
    detail = detail,
    state = state,
    fred_behavior = why,
    doc = doc,
  }
end

-- index.txt: keys of every mode, and each ex command's abbreviation.
local index = read('runtime/doc/index.txt')
local abbrev = {}
local mode
for line in index:gmatch('[^\n]*') do
  local section = line:match('^(%d[%.%d]*)%.? +%S')
  if section and line:find('*', 1, true) then
    mode = KEY_MODES[section:gsub('%.$', '')]
  end
  local tag, key, rest = line:match('^|([^|]+)|\t+([^\t]+)\t*(.*)$')
  if tag then
    if tag:sub(1, 1) == ':' then
      abbrev[tag:sub(2)] = key
    elseif mode then
      local note, desc = rest:match('^(%d?)%s*(.*)$')
      local detail = ({ ['1'] = 'motion', ['2'] = 'change' })[note] or ''
      add('key', mode, key, detail, desc)
    end
  end
end

-- ex_cmds.lua: name, flags and handler.
local ex = dofile(source .. '/src/nvim/ex_cmds.lua')
for _, c in ipairs(ex.cmds) do
  local min = abbrev[c.command] or c.command
  add('ex', '', c.command, min .. ' ' .. (c.addr_type or ''), c.func)
end

-- options.lua: name, abbreviation, scope and type.
local opts = dofile(source .. '/src/nvim/options.lua')
for _, o in ipairs(opts.options) do
  local detail = table.concat({ o.abbreviation or '', table.concat(o.scope, '+'), o.type }, ' ')
  local desc = o.short_desc
  if type(desc) == 'function' then
    desc = desc()
  end
  desc = desc and (desc:match('^N_%("(.*)"%)$') or desc)
  add('option', '', o.full_name, detail, desc or '')
  if EXPR_TYPES[o.type] and rows[#rows].state == 'missing' then
    rows[#rows].state, rows[#rows].fred_behavior = 'deferred', 'expression hook (Vimscript/Lua)'
  end
end

-- auevents.lua: events, with the comment beside each as its description.
local events = read('src/nvim/auevents.lua')
for name, value, comment in events:gmatch('\n%s+([%w]+) = (%a+),?%s*%-?%-?%s*([^\n]*)') do
  add('event', value == 'true' and 'window' or '', name, '', comment)
end

-- Keep what the ledger already says about each row.
local old = {}
local f = io.open(output)
if f then
  f:read('l')
  for line in f:lines() do
    local fields, i = {}, 1
    while i <= #line + 1 do
      local v
      if line:sub(i, i) == '"' then
        local j = i + 1
        v = ''
        while true do
          local q = line:find('"', j, true)
          v = v .. line:sub(j, q - 1)
          if line:sub(q + 1, q + 1) == '"' then
            v, j = v .. '"', q + 2
          else
            i = q + 2
            break
          end
        end
      else
        local c = line:find(',', i, true) or #line + 1
        v, i = line:sub(i, c - 1), c + 1
      end
      fields[#fields + 1] = v
    end
    old[fields[1] .. '\0' .. fields[2] .. '\0' .. fields[3]] = fields
  end
  f:close()
end

local function csv(v)
  v = tostring(v or ''):gsub('\n', ' ')
  if v:find('[,"]') then
    return '"' .. v:gsub('"', '""') .. '"'
  end
  return v
end

local out = assert(io.open(output, 'w'))
out:write('kind,mode,symbol,detail,state,fred_behavior,doc\n')
for _, r in ipairs(rows) do
  local prev = old[r.kind .. '\0' .. r.mode .. '\0' .. r.symbol]
  if prev then
    r.state, r.fred_behavior = prev[5], prev[6]
  end
  out:write(
    table.concat({ csv(r.kind), csv(r.mode), csv(r.symbol), csv(r.detail), csv(r.state), csv(r.fred_behavior), csv(r.doc) }, ',')
      .. '\n'
  )
end
out:close()
