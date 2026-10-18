# wezfont

[English](README.md) | **中文**

[WezTerm](https://wezterm.org) 的字体选择 TUI。

挑主字体、维护回退链，而且**选中即预览**：结果写进 `~/.wezterm-font.lua`，
再只更新 `~/.wezterm.lua` 的 mtime 触发 WezTerm 热重载 —— 右边样张当场就用新字体渲染。

## 用法

```bash
wezfont              # 打开 TUI
wezfont --info       # 显示检测到的路径、字体数、当前主字体和回退链
wezfont --list       # 列出可用字体
wezfont --set "Consolas"
wezfont --fallback "Microsoft YaHei,Cascadia Code"
wezfont --scan       # 重新扫描系统字体并刷新缓存
```

### 按键

| 键 | 作用 |
| --- | --- |
| `Tab` | 在「主字体」和「回退链」之间切换 |
| `↑↓` / 直接打字 | 移动 / 模糊过滤 |
| `Enter` | 应用主字体，或加入/移除回退项 |
| `Shift+J` / `Shift+K` | 调整选中项在回退链里的位置 |
| `r` | 重新扫描字体 |
| `Esc` | 清空过滤 / 退出 |

## 写出的文件

`~/.wezterm-font.lua`：

```lua
return {
  font = "0xProto Nerd Font Mono",
  fallback = { "Microsoft YaHei", "Cascadia Code" },
}
```

`~/.wezterm.lua` 会读这个文件，拼成 `wezterm.font_with_fallback` 链。删掉该文件即恢复到
配置里手写的字体。

## 安装

```bash
wezext install wezfont
# 或从源码：
wezext install --build wezfont
```

## WSL

字体始终是 **Windows 侧**的（WezTerm GUI 在 Windows 上渲染），所以从 WSL shell 里启动
`wezfont` 也能正常工作 —— 会自动解析到 Windows 的家目录。

## 构建

```bash
cargo build --release
# -> target/release/wezfont.exe
```
