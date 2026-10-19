# ESLint + Prettier 接入计划

## Context（背景）

EdgeTerm 前端目前没有任何 lint / format 工具：无 ESLint、无 Prettier，质量关卡只有 `tsc --noEmit`（类型）和 vitest（单测），代码风格纯靠约定维护。本次按已确认的最小方案接入两个工具，不影响现有构建流程。

摸底结论（支撑方案设计）：

- 103 个 TS/TSX 文件约 3 万行，另有 `src/styles.css` 4882 行
- 现有代码风格（双引号、分号、2 空格、尾随逗号、80 列）恰好是 Prettier 默认值 → 全量格式化 diff 预计很小（用户已确认一次性全量格式化）
- `tsc` 已开启 strict + noUnusedLocals/noUnusedParameters → ESLint 不启用 type-aware 规则，不重复把关类型
- 测试文件显式 import vitest API（非 globals 模式）→ 无需特殊配置
- node_modules 中不存在任何 eslint/prettier 包，干净的接入起点
- Rust 侧已有 cargo fmt/clippy 默认规则，本次不动

## 实施步骤

### 1. 安装 devDependencies

```
prettier
eslint  @eslint/js  typescript-eslint
eslint-plugin-react-hooks
eslint-plugin-react-refresh
eslint-config-prettier
```

安装最新稳定版。实施第一步先验证 typescript-eslint 解析器与项目 TypeScript 7 的兼容性（本项目不启用 type-aware 规则，纯语法解析，风险低；若解析器不兼容则当场降级处理并如实报告）。

### 2. 新增配置文件

**`eslint.config.js`**（flat config，ESLint 9 标准）：

- 全局 ignores：`dist/`、`src-tauri/`（Rust 侧有自己的工具链）、`coverage/`、`docs/`
- 基础规则集：`@eslint/js` recommended + `typescript-eslint` recommended（非 type-aware）+ `eslint-config-prettier`（关闭所有与格式化冲突的规则）
- `src/**/*.{ts,tsx}` 追加：`eslint-plugin-react-hooks` recommended（useEffect 依赖数组等正确性检查）+ `eslint-plugin-react-refresh`（组件文件导出约束，保 HMR）
- `scripts/**/*.mjs`：声明 node 环境 globals

**`.prettierrc.json`**：显式固化默认值（`semi: true`、`singleQuote: false`、`printWidth: 80`、`tabWidth: 2`、`trailingComma: "all"`），防止未来 Prettier 默认值漂移导致风格突变。

**`.prettierignore`**：`dist/`、`src-tauri/`、`docs/`、`.trae/`、`package-lock.json`。

### 3. package.json 新增脚本

```
"lint": "eslint ."
"lint:fix": "eslint . --fix"
"format": "prettier --write ."
"format:check": "prettier --check ."
```

不改动现有 dev / build / test / typecheck 任何脚本。

### 4. 一次性全量格式化 + lint 清零

1. `npm run format` → 一次性格式化全部源文件（预期 diff 集中在少数文件、行内小改）
2. `npm run lint:fix` → 自动修复可修复项；剩余告警逐条最小修复（预期很少，常见为 react-hooks/exhaustive-deps 依赖数组、react-refresh 导出混合），不做任何重构
3. `src/vite-env.d.ts`、`src/zmodem-js.d.ts` 等环境声明文件若误报，在 eslint config 中局部放宽，不改声明内容

### 5. 编辑器配置（零风险加分项）

- `.vscode/extensions.json`：推荐 ESLint 与 Prettier 官方扩展
- `.vscode/settings.json`：保存时自动格式化 + ESLint 自动修复，让规范自动生效

## 验证

1. `npm run lint` → 0 error 0 warning
2. `npm run format:check` → 全部通过
3. `npm run typecheck` → 通过（确认格式化未破坏类型）
4. `npm test` → 262 个测试全部通过
5. `npm run build` → 构建成功
6. 抽查 `git diff`：格式化改动应集中在少量文件，无大面积重排

## 涉及文件

- 新增：`eslint.config.js`、`.prettierrc.json`、`.prettierignore`、`.vscode/extensions.json`、`.vscode/settings.json`
- 修改：`package.json`（scripts + devDependencies）；格式化触及的 `src/**` 与根目录 `vite.config.ts`、`vitest.config.ts`、`index.html`

## 明确不做

- 不加 git 提交钩子（husky / lint-staged）
- 不改 GitHub Actions
- 不动 Rust 侧
- 不启用 type-aware lint（tsc 已覆盖）
- 不引入任何新风格约定（全用 Prettier 默认值）
