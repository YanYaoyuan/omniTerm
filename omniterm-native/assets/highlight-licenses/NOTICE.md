# Terminal highlight palette sources

omniTerm maps selected upstream colors to its own terminal semantic roles. This is an adaptation, not a complete editor theme. Light/dark variants are selected for the current background, and colors below a 4.5:1 contrast ratio are adjusted locally toward black or white. Nord uses this adjustment for light backgrounds. The Moba-inspired classic palette and theme selection are omniTerm's own implementation.

Upstream source file URLs and Git blob SHAs (retrieved 2026-11-17):

- dracula/dracula-theme: https://github.com/dracula/dracula-theme/blob/main/README.md (blob `a737b59ff9e4944f0eac8a95a7defecfefb066c1`); license: `Dracula-MIT.txt`.
- folke/tokyonight.nvim: https://github.com/folke/tokyonight.nvim/blob/main/lua/tokyonight/colors/storm.lua (blob `dfba7a1b2277628e6a9b87ad4a1dad8387b4d2f6`); license: `TokyoNight-Apache-2.0.txt`.
- catppuccin/palette: https://github.com/catppuccin/palette/blob/main/palette.json (blob `2a5d25cb0b00033bb049d11ab12c8bed478bf4e8`); license: `Catppuccin-MIT.txt`.
- nordtheme/nord: https://github.com/nordtheme/nord/blob/develop/src/nord.css (blob `e492c4f2a717544c5414d0241601663e70b93d84`); license: `Nord-MIT.txt`.
- gruvbox-community/gruvbox: https://github.com/gruvbox-community/gruvbox/blob/master/colors/gruvbox.vim (blob `915ea194a689dbf41e92cc7595458b228c07910d`); license: `Gruvbox-MIT.txt`.
- atom/one-dark-syntax: https://github.com/atom/one-dark-syntax/blob/master/styles/colors.less (blob `346a676857ab2b8004d9881ad9897bbc09b06f27`); license: `Atom-One-MIT.txt`.
- rose-pine/neovim: https://github.com/rose-pine/neovim/blob/main/lua/rose-pine/palette.lua (blob `cfefd6592cd757c5ce15c0f19b1c7ec88739d9b7`); license: `Rose-Pine-MIT.txt`.
- altercation/solarized: https://github.com/altercation/solarized/blob/master/README.md (blob `d18ee9450251ea1b9a02ebd4d6fce022df9eb5e4`); license: `Solarized-MIT.txt`.

Additional light palettes: Tokyo Night Day from https://github.com/folke/tokyonight.nvim/blob/main/extras/wezterm/tokyonight_day.toml (blob `ab7b3d6e044e988d66e58d45c11b6e0c4f91f9eb`); Atom One Light from https://github.com/atom/one-light-syntax/blob/master/styles/colors.less (blob `ca513b9dc5a87aae583f77efbc516c92c221012d`). Atom's license above covers both dark and light variants. Dracula Alucard, Catppuccin Latte, Rosé Pine Dawn and Gruvbox light colors are in the corresponding source files above.

Tokyo Night is by folke and contributors (Apache-2.0). The other adapted palettes are under their included MIT licenses; original copyright notices are retained in those files. Changes: semantic role selection and mapping, background-dependent variant selection, and local contrast adjustment by omniTerm contributors, 2026.
