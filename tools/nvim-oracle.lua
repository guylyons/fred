-- nvim-oracle.lua: what real Neovim does with TEXT after typing KEYS.
--
--   nvim --clean --headless -l tools/nvim-oracle.lua TEXT KEYS
-- KEYS uses Fred's (and Vim's) notation: <Esc>, <C-r>, <CR>, <BS>….
-- The last line printed is JSON: {"text", "line" (0-based), "col" (byte), "mode"}.
-- (Neovim may echo `:` commands on lines before it.)

local text, keys = arg[1] or '', arg[2] or ''
vim.o.showmode = false
vim.api.nvim_buf_set_lines(0, 0, -1, false, vim.split(text, '\n', { plain = true }))
vim.api.nvim_win_set_cursor(0, { 1, 0 })

-- Runs as the last key (<Cmd> keeps the mode), so it sees where KEYS left off.
function _G.FredOracleReport()
  local pos = vim.api.nvim_win_get_cursor(0)
  io.write(vim.json.encode({
    text = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), '\n'),
    line = pos[1] - 1,
    col = pos[2],
    mode = vim.api.nvim_get_mode().mode,
  }), '\n')
end

vim.api.nvim_feedkeys(vim.keycode(keys .. '<Cmd>lua FredOracleReport()<CR>'), 'xt', false)
