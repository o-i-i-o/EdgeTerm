# Oxlint + tsgolint 替换 ESLint 计划

## Context（背景）

上一轮接入的 ESLint 因 TypeScript 7 不再提供 JS 编译器 API，被迫用 @babel/eslint-parser 降级，付出两个代价：`no-unused-vars` 全关（babel 看不见类型引用，179 处误报）、typescript-eslint 不可用（官方已将 TS7 支持标记 not planned）。

oxlint 1.88（Rust 原生解析 TS/TSX，不依赖 TS API）+ `oxlint-tsgolint`（基于 TS 7.0.2 原生编译器的 type-aware 引擎，2026-07 转正）正好补上这个断层。现有规则集每一条都有原生对等物，包括被迫关闭的 no-unused-vars（原生 TS 解析后可重新开启）。

用户决策：**完全替换 ESLint**；**严格度与现状等价**（Compiler 四条规则保持关闭）；Prettier 保留不动（oxfmt 仍是 beta，不折腾）。

## 实施步骤

### 1. 依赖变更

- 卸载（8 个）：`eslint`、`@eslint/js`、`@babel/eslint-parser`、`@babel/core`、`eslint-plugin-react-hooks`、`eslint-plugin-react-refresh`、`eslint-config-prettier`、`globals`
- 安装：`oxlint`、`oxlint-tsgolint`
- 保留：`prettier`

### 2. 配置替换

- 删除 `eslint.config.js`
- 新增 `.oxlintrc.json`，规则映射（现状等价）：
  - 核心 recommended → oxlint correctness 类目（默认）
  - `typescript/no-unused-vars: "error"`——重新开启（本轮最大收益），按 tsc 语义配置 `^_` 忽略模式
  - `react/rules-of-hooks: "error"`、`react/exhaustive-deps: "warn"`（oxlint 原生，非 react-hooks 插件名）
  - `react/only-export-components: "warn"`（含 allowConstantExport）
  - Compiler 四条（`refs` / `purity` / `set-state-in-effect` / `immutability`）显式关闭，注明后续可开
  - `ignorePatterns`: `dist/`、`src-tauri/`、`coverage/`、`docs/`、`.trae/`

### 3. 迁移现有 10 处 disable 注释

oxlint 支持 eslint 风格 disable 指令，但规则名不同：`react-hooks/exhaustive-deps` → `react/exhaustive-deps`、`react-refresh/only-export-components` → `react/only-export-components`（oxlint 是否接受旧名前缀以实测为准）。3 处 `no-control-regex` 注释同名应直接生效。以 oxlint 报出的 "unused disable directive" 清单逐处修正，涉及：AuthPromptDialog、SearchOverlay（×3）、ShortcutsDialog、FilerPanel、SenderPanel、ContextMenu、TerminalPane、SessionPanel。

### 4. package.json 脚本

- `"lint": "oxlint ."`、`"lint:fix": "oxlint --fix ."`
- `format` / `format:check` 不变

### 5. tsgolint 落地（分支逻辑，避免死代码）

试运行 `oxlint --type-aware .` 看 type-aware 规则（no-floating-promises 等）的命中量：

- **命中少（≤10 处）**：逐条修复或带原因禁用，`--type-aware` 直接并入 `"lint"` 脚本——完整兑现 tsgolint 能力
- **命中多**：新增独立脚本 `"lint:type-aware": "oxlint --type-aware ."` 作为后续修复轮入口，默认 `lint` 保持现状等价零新告警

无论哪个分支，oxlint-tsgolint 都有实际用途，不产生死依赖。

## 验证

1. `npm run lint` → 0 error 0 warning
2. `npm run format:check` → 通过（Prettier 未动，不应有格式 diff）
3. `npm run typecheck` → 通过
4. `npm test` → 262 全过
5. `npm run build` → 成功
6. `time npm run lint` 性能对照（预期从分钟级降到秒级）
7. `git diff` 抽查：本轮改动应仅限依赖/配置/注释行，无业务逻辑变化

## 涉及文件

- 新增：`.oxlintrc.json`
- 删除：`eslint.config.js`
- 修改：`package.json`；10 处 disable 注释所在源文件的注释行（规则名或指令写法）
- 不动：`.prettierrc.json`、`.prettierignore`、全部业务逻辑
