# wezfont

wezterm 的字体选择 TUI：主字体 + fallback 回退链，选中即写 `~/.wezterm-font.lua`，
并只更新 `~/.wezterm.lua` 的 mtime 触发 wezterm 热重载 —— 样张当场变化，即实时预览。

## 用法

```bash
wezfont              # 打开 TUI
wezfont --info       # 看检测到的路径 / 字体数 / 当前字体
wezfont --list       # 列出字体
wezfont --set "Consolas"
wezfont --fallback "Microsoft YaHei,Cascadia Code"
wezfont --scan       # 重新扫描系统字体
```

TUI 里：

| 键 | 作用 |
|---|---|
| `Tab` | 主字体 ⇄ Fallback 回退链 |
| `↑↓` / 打字 | 选择 / 过滤 |
| `Enter` | 应用（主字体页）或加入/移除回退链（Fallback 页） |
| `Shift+J/K` | 调整回退链顺序 |
| `r` | 重新扫描字体 |
| `Esc` | 清过滤 / 退出 |

## 安装

```bash
wezext install wezfont
# 或者从源码：
wezext install --build wezfont
```

## WSL

字体始终是 **Windows 侧**的（wezterm GUI 在 Windows 上渲染），
所以从 WSL shell 里启动也行，家目录会自动解析到 `C:\Users\<你>`。
