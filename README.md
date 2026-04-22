# DNFAutoFire (Rust)

这个仓库已重构为 **Rust 版本的 DNFAutoFire**，核心行为与原 Python 项目一致：

- 仅在目标窗口（默认关键字：`地下城与勇士`、`DNF`）前台时工作
- 按住配置中的按键时进行连发
- 支持一键连招：按一次触发键，按顺序执行多个按键
- 提供 Win32 原生 GUI 配置界面，支持图形化编辑配置与启动/停止
- GUI 支持系统托盘常驻、单实例保护，并在托盘图标上区分开启/暂停/关闭状态
- 按 `Esc` 退出
- 使用 `configs.json` 管理多套配置（保存/删除/设置默认）

## 运行

```bash
cargo run -- run
```

如果省略子命令，默认等价于 `run`。

打开 GUI：

```bash
cargo run -- gui
```

GUI 特性：

- 左侧管理配置列表，可新建、保存、删除、设为默认
- 右侧提供完整键盘视图，点击键帽即可切换是否加入连发
- 右侧同时可编辑普通连发参数、目标窗口关键字和一键连招
- 连招使用独立弹窗编辑，支持按步骤添加、删除、上移、下移
- 关闭主窗口或最小化时会缩到系统托盘，托盘菜单可显示/隐藏窗口、启动/停止连发、退出程序
- 托盘图标会区分 `连发已开启 / 输入法暂停 / 连发已关闭`
- GUI 为单实例，重复启动时会直接拒绝并提示使用现有托盘实例
- 运行中会禁用配置编辑，只保留停止按钮，避免运行态配置漂移
- 状态栏显示 `未运行 / 运行中 / 输入法暂停 / 已停止`

## 配置命令

列出配置：

```bash
cargo run -- config list
```

查看配置：

```bash
cargo run -- config show --profile default
```

保存配置：

```bash
cargo run -- config save --name my-dnf --keys J,P,L,H --repeat-interval-ms 1 --press-duration-ms 1 --poll-interval-ms 1 --windows 地下城与勇士,DNF --set-default
```

添加一键连招：

```bash
cargo run -- config add-combo --profile my-dnf --name combo1 --trigger-key U --sequence-keys A,S,D,F --step-interval-ms 80 --press-duration-ms 1
```

删除一键连招：

```bash
cargo run -- config remove-combo --profile my-dnf --name combo1
```

删除配置：

```bash
cargo run -- config delete --name my-dnf
```

设置默认配置：

```bash
cargo run -- config set-default --name default
```

## 默认配置

首次运行会自动生成 `configs.json`，默认内容如下：

- `enabled_keys`: `J`, `P`, `L`, `H`
- `repeat_interval_ms`: `1`
- `press_duration_ms`: `1`
- `poll_interval_ms`: `1`
- `target_windows`: `地下城与勇士`, `DNF`
- `combos`: `[]`

## 一键连招配置说明

每个连招包含这些字段：

- `name`: 连招名称
- `trigger_key`: 触发键，按下一次后执行整套连招
- `sequence_keys`: 按顺序执行的按键列表，支持重复键
- `step_interval_ms`: 每一步之间的时间间隔
- `press_duration_ms`: 每一步按下到释放的持续时间
