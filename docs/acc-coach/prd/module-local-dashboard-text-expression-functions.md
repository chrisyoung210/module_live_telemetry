# module_local_dashboard text widget 表达式内置函数支持需求

> 目标模块：`module_local_dashboard`
>
> 需求方：`acc-coach`
>
> 需求类型：功能新增（表达式求值器扩展）
>
> 状态：等待开发
>
> 前置依赖：无（本需求不依赖协议类型变更）
>
> 关联规范：`docs/acc-coach/public-protocol/text-expression-function-spec.md`（以下简称"函数规范"）

---

## 一、背景

text widget 的 `textTemplate` 中可嵌入 `{{expr:...}}` 表达式，对 telemetry 值做运算后输出。`module_local_dashboard` 内置运行时表达式求值器 `src-ui/features/local-dashboard-overlay/textExpression.ts`，负责在 overlay 每帧渲染时计算每个 text widget 的最终文本。

当前求值器支持字面量、`{field}` 占位符、四则运算、比较、逻辑、三元运算，**不支持函数调用**。`acc-coach` 侧的 Dashboard Designer 即将新增"insert function"按钮，允许用户在文本模板中插入 `abs(...)` / `round(...)` 等内置函数。运行时求值器必须同步支持这些函数，否则设计时预览正确但运行时 overlay 显示空串或报错。

本需求要求 `module_local_dashboard` 按函数规范扩展其表达式求值器，与 `acc-coach` 设计时求值器保持语义一致。

---

## 二、现状分析

### 2.1 tokenizer 阻塞函数名

`textExpression.ts` 的 `tokenize` 函数（第 31-119 行）在读取标识符时（第 75-93 行），对**非保留字标识符**直接抛错：

```typescript
// textExpression.ts:75-93（当前实现）
if (/[A-Za-z_$]/.test(ch)) {
  const start = index++;
  while (
    index < expression.length &&
    /[A-Za-z0-9_$]/.test(expression[index])
  )
    index += 1;
  const identifier = expression.slice(start, index);
  if (identifier === "true") tokens.push({ type: "value", value: true });
  else if (identifier === "false") tokens.push({ type: "value", value: false });
  else if (identifier === "null") tokens.push({ type: "value", value: null });
  else if (identifier === "undefined") tokens.push({ type: "value", value: undefined });
  else if (identifier === "NaN") tokens.push({ type: "value", value: NaN });
  else if (identifier === "Infinity") tokens.push({ type: "value", value: Infinity });
  else throw new Error(`Unknown identifier: ${identifier}`);   // ← 阻塞点
  continue;
}
```

这意味着 `round` / `abs` 这类函数名在 tokenize 阶段就被拒绝，根本无法进入 parser。

### 2.2 Token 类型缺失

当前 `Token` 联合类型（第 5-10 行）没有 `identifier` 类别：

```typescript
type Token =
  | { type: "value"; value: ExpressionValue }
  | { type: "operator"; value: string }
  | { type: "paren"; value: "(" | ")" }
  | { type: "question" }
  | { type: "colon" };
```

保留字（`true`/`false`/`null`/`undefined`/`NaN`/`Infinity`）被直接转为 `value` token，非保留字无对应 token 类型。

### 2.3 Parser 不支持函数调用

`Parser` 类（第 124-249 行）的 `unary` 方法（第 221-235 行）是"主表达式"解析入口，仅处理：

- 一元运算符 `!` / `-` / `+`
- `value` token（字面量）
- 括号表达式 `(...)`

不识别 `identifier(args)` 形式。

### 2.4 resolveControlText 调用链

`dashboardRenderer.tsx:336` 的 `resolveControlText` 在检测到 `{{expr:...}}` 时调用 `evaluateTextExpression`，外层 `try/catch` 吞掉异常并返回空串（第 352-354 行）。因此求值失败不会让 overlay 崩溃，但用户会看到空白文本而无法定位问题——这正是本需求要避免的：必须让函数调用在运行时正确求值。

---

## 三、目标实现

### 3.1 扩展 Token 类型

新增 `identifier` token 类型，用于承载非保留字标识符（函数名或字段名）：

```typescript
type Token =
  | { type: "value"; value: ExpressionValue }
  | { type: "identifier"; value: string }        // ← 新增
  | { type: "operator"; value: string }
  | { type: "paren"; value: "(" | ")" }
  | { type: "question" }
  | { type: "colon" };
```

### 3.2 修改 tokenizer

将第 75-93 行的"非保留字即抛错"改为"产出 identifier token"：

```typescript
if (/[A-Za-z_$]/.test(ch)) {
  const start = index++;
  while (
    index < expression.length &&
    /[A-Za-z0-9_$]/.test(expression[index])
  )
    index += 1;
  const identifier = expression.slice(start, index);
  if (identifier === "true") tokens.push({ type: "value", value: true });
  else if (identifier === "false") tokens.push({ type: "value", value: false });
  else if (identifier === "null") tokens.push({ type: "value", value: null });
  else if (identifier === "undefined") tokens.push({ type: "value", value: undefined });
  else if (identifier === "NaN") tokens.push({ type: "value", value: NaN });
  else if (identifier === "Infinity") tokens.push({ type: "value", value: Infinity });
  else tokens.push({ type: "identifier", value: identifier });   // ← 不再抛错
  continue;
}
```

> **注意**：`textExpression.ts` 的求值入口 `evaluateTextExpression`（第 266 行）在调用 `tokenize` 之前，已经用正则把所有 `{field}` 占位符替换为 JSON 字面量（第 275-285 行）。因此 tokenizer 实际不会遇到"字段名标识符"——字段名都被替换成字面量数字/字符串了。`identifier` token 在当前架构下**只会**承载函数名。这与 `acc-coach` 设计时求值器不同（后者把字段名作为 context 变量传入）。两边语义仍然一致，详见函数规范第 4.1 节。

### 3.3 修改 Parser 解析函数调用

在 `unary` 方法（第 221-235 行）的"非一元运算符"分支中，增加对 `identifier` token 的处理：

```typescript
private unary(): ExpressionValue {
  if (this.operator("!")) return !truthy(this.unary());
  if (this.operator("-")) return -number(this.unary());
  if (this.operator("+")) return number(this.unary());
  const token = this.take();

  // 字面量
  if (token?.type === "value") return token.value;

  // 函数调用：identifier 后紧跟 "("
  if (token?.type === "identifier") {
    const next = this.peek();
    if (next?.type === "paren" && next.value === "(") {
      this.take(); // 消耗 "("
      const args: ExpressionValue[] = [];
      // 空参数列表
      const afterOpen = this.peek();
      if (afterOpen?.type === "paren" && afterOpen.value === ")") {
        this.take();
      } else {
        while (true) {
          args.push(this.conditional());
          const sep = this.take();
          if (sep?.type === "paren" && sep.value === ")") break;
          if (sep?.type !== "operator" || sep.value !== ",")
            throw new Error("Expected ',' or ')' in function arguments");
        }
      }
      const fn = BUILTIN_FUNCTIONS[token.value];
      if (!fn) throw new Error(`Unknown function: ${token.value}`);
      return fn(args);
    }
    // identifier 后非 "("：当前架构下不会出现（字段名已被替换为字面量）
    // 但为稳健起见，按未知字段处理
    throw new Error(`Unknown telemetry field: ${token.value}`);
  }

  // 括号表达式
  if (token?.type === "paren" && token.value === "(") {
    const value = this.conditional();
    const close = this.take();
    if (close?.type !== "paren" || close.value !== ")")
      throw new Error("Expected ')'");
    return value;
  }

  throw new Error("Unexpected token");
}
```

### 3.4 新增内置函数表

在 `textExpression.ts` 顶部（建议放在 `OPERATORS` 常量之后、`tokenize` 函数之前）新增：

```typescript
type ExpressionValue = string | number | boolean | null | undefined;

const BUILTIN_FUNCTIONS: Record<
  string,
  (args: ExpressionValue[]) => ExpressionValue
> = {
  abs: (args) => {
    if (args.length !== 1) throw new Error("Invalid arguments for abs");
    return Math.abs(Number(args[0]));
  },
  round: (args) => {
    if (args.length !== 2) throw new Error("Invalid arguments for round");
    const x = Number(args[0]);
    const n = Math.floor(Number(args[1]));
    if (!Number.isFinite(n) || n < 0)
      throw new Error("Invalid arguments for round: n must be >= 0");
    if (!Number.isFinite(x)) return x;
    return Number(x.toFixed(n));
  },
};
```

> 函数实现必须与函数规范第 4.3 节给出的参考实现**语义等价**，包括参数校验、错误信息文本、返回值。错误信息文本用于 `acc-coach` 设计时与运行时交叉比对，必须一字不差。

---

## 四、不改动的内容

| 文件 / 函数 | 原因 |
|-------------|------|
| `dashboardRenderer.tsx` 的 `resolveControlText`（第 336 行） | 求值失败仍返回空串，行为不变 |
| `evaluateTextExpression` 的占位符替换逻辑（第 266-285 行） | `{field}` → 字面量的替换规则不变 |
| `formatExpressionValue`（第 251-264 行） | format 规则不变 |
| `telemetryFormat.ts` | 与函数调用无关 |
| `DashboardControl` / `textTemplate` 协议字段 | 不涉及协议变更 |
| `module_dashboard_protocol` | 无需任何改动 |

---

## 五、验收标准

1. `tokenize("abs(-3.5)")` 不再抛 `Unknown identifier`，返回包含 `identifier`、`paren`、`value` 的 token 序列
2. `evaluateTextExpression("abs(-3.5)", {})` 返回 `"3.5"`
3. `evaluateTextExpression("round(3.14159, 2)", {})` 返回 `"3.14"`
4. `evaluateTextExpression("round(abs(-3.14159), 2)", {})` 返回 `"3.14"`（嵌套）
5. `evaluateTextExpression("unknownFn(1)", {})` 抛 `Unknown function: unknownFn`
6. `evaluateTextExpression("abs(1, 2)", {})` 抛 `Invalid arguments for abs`
7. `evaluateTextExpression("round(1, -1)", {})` 抛 `Invalid arguments for round: n must be >= 0`
8. 函数规范第六节的全部 17 个一致性测试用例通过
9. 旧布局（不含函数调用的 `textTemplate`）渲染行为完全不变
10. `resolveControlText` 在求值成功时返回正确文本，在求值失败时仍返回空串（不崩溃）

---

## 六、测试要求

- 在 `textExpression.ts` 同目录或测试目录下新增针对内置函数的单元测试，覆盖函数规范第六节的全部用例
- 保留现有 `textExpression` 测试用例不动，确保未引入回归
- 嵌套调用、与字符串拼接、与三元运算混合的用例必须有测试

---

## 七、与 acc-coach 的协同

- `acc-coach` 侧的 Dashboard Designer UI 改动（insert function 按钮、函数选择弹窗、5 行布局）由 `acc-coach` 自行实现，不影响本需求
- `acc-coach` 侧的设计时求值器（`DashboardDesignerView.tsx` 的 `ExpressionParser`）同步支持相同的函数表，详见 `acc-coach` 内部 plan 文档
- 两边实现完成后，使用包含函数调用的同一份 layout 在 Dashboard Designer 预览与 overlay 实际渲染中对比，结果必须一致

---

## 八、时间安排

无硬性时间要求。建议与 `acc-coach` 的 UI 改动同步上线，避免出现"设计时可插入函数但运行时无法求值"的中间状态。

如果 `acc-coach` 先发布 UI 改动，用户插入的函数调用在运行时 overlay 会显示空串（求值失败被 catch 吞掉），不会崩溃但体验不佳。因此建议两边同批次发布。
