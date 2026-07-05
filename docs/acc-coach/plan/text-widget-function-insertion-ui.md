# acc-coach Text Widget 函数插入功能实现方案

> 目标模块：`acc-coach`（本仓库自身）
>
> 文档类型：内部实现方案 / 开发计划
>
> 状态：待开发
>
> 关联规范：`docs/acc-coach/public-protocol/text-expression-function-spec.md`（以下简称"函数规范"）
>
> 关联 PRD：`docs/acc-coach/prd/module-local-dashboard-text-expression-functions.md`（对 `module_local_dashboard` 的需求）

---

## 一、目标

在 Dashboard Designer 编辑 text widget 的 "Text Content" 弹窗中，新增 "insert function" 入口，允许用户将内置函数（`abs` / `round`）插入到文本模板中，并在弹窗中实时预览求值结果。

### 1.1 用户场景

用户编辑一个 text widget，想显示"车速保留 1 位小数"：

1. 打开 text widget 的 Text Content 弹窗
2. 点击 **insert function** 按钮 → 弹出函数选择列表
3. 选择 `round(x, n)` → 文本框插入 `round(|, 2)`，光标选中 `|` 占位符
4. 点击 **insert telemetry item** 按钮 → 弹出 telemetry channel 列表
5. 选择 `speedKmh` → `|` 被替换为 `{speedKmh}`，文本变为 `round({speedKmh}, 2)`
6. 在 "Example raw value" 中输入 `164.389`，Preview 显示 `164.39`
7. 点击 OK 保存

### 1.2 功能边界

- 仅支持函数规范第三节定义的内置函数（当前为 `abs` / `round`，未来扩展时更新函数表）
- 不提供 UDF
- 函数插入仅作用于 text widget 的 Text Content 弹窗（不涉及 dynamic properties 中的 `visibleExpression` / `backgroundColorExpression` / `textColorExpression` 等其他表达式编辑器，本次不改）

---

## 二、UI 改动

### 2.1 当前布局（4 行）

`DashboardDesignerView.tsx` 第 3226-3327 行的 `textContentOpen` 弹窗：

```
┌────────────────────────────────────────────┐
│  Text Content                              │  标题
├────────────────────────────────────────────┤
│  [大文本框，rows=10]                        │  第 1 行
├────────────────────────────────────────────┤
│  [Insert telemetry item] [Example raw value] [Format] │  第 2 行
├────────────────────────────────────────────┤
│  [Preview]                                 │  第 3 行
├────────────────────────────────────────────┤
│                       [OK] [Cancel]        │  第 4 行
└────────────────────────────────────────────┘
```

第 2 行结构（第 3249-3296 行）：

```tsx
<div className={styles.telemetryInsertRow}>
  <button onClick={openTelemetryPicker}>Insert telemetry item</button>
  <label>Example raw value <input .../></label>
  <label>Format <select ...>{TEXT_FORMAT_OPTIONS.map(...)}</select></label>
</div>
```

### 2.2 目标布局（5 行）

```
┌────────────────────────────────────────────┐
│  Text Content                              │  标题
├────────────────────────────────────────────┤
│  [大文本框，rows=10]                        │  第 1 行
├────────────────────────────────────────────┤
│  [Insert function] [Insert telemetry item] │  第 2 行
├────────────────────────────────────────────┤
│  [Example raw value]                       │  第 3 行
├────────────────────────────────────────────┤
│  [Format]               [Preview]          │  第 4 行
├────────────────────────────────────────────┤
│                       [OK] [Cancel]        │  第 5 行
└────────────────────────────────────────────┘
```

### 2.3 JSX 改动

将第 3249-3310 行的 `telemetryInsertPanel` 整体重构为：

```tsx
<div className={styles.telemetryInsertPanel}>
  {/* 第 2 行：两个插入按钮 */}
  <div className={styles.telemetryInsertRow}>
    <button
      className={styles.button}
      type="button"
      onClick={openFunctionPicker}
    >
      Insert function
    </button>
    <button
      className={styles.button}
      type="button"
      onClick={openTelemetryPicker}
    >
      Insert telemetry item
    </button>
  </div>

  {/* 第 3 行：Example raw value 独占一行 */}
  <label className={styles.field}>
    Example raw value
    <input
      className={styles.input}
      value={telemetryExampleValue}
      onChange={(event) => setTelemetryExampleValue(event.target.value)}
      disabled={!selectedTelemetryItem}
    />
  </label>

  {/* 第 4 行：Format + Preview 同行 */}
  <div className={styles.telemetryInsertRow}>
    <label className={styles.field}>
      Format
      <select
        className={styles.select}
        value={telemetryFormat}
        onChange={(event) => {
          const nextFormat = event.target.value;
          const oldDefault = selectedTelemetryItem
            ? telemetryToken(selectedTelemetryItem, telemetryFormat)
            : "";
          const nextDefault = selectedTelemetryItem
            ? telemetryToken(selectedTelemetryItem, nextFormat)
            : "";
          setTelemetryFormat(nextFormat);
          setTextDraft((current) =>
            current.trim() === oldDefault ? nextDefault : current,
          );
        }}
        disabled={!selectedTelemetryItem}
      >
        {TEXT_FORMAT_OPTIONS.map((format) => (
          <option key={format || "raw"} value={format}>
            {format || "Raw"}
          </option>
        ))}
      </select>
    </label>
    <label className={styles.field}>
      Preview
      <input
        className={styles.input}
        value={telemetryPreview.value}
        readOnly
      />
      {telemetryPreview.error ? (
        <span className={styles.fieldError}>{telemetryPreview.error}</span>
      ) : null}
    </label>
  </div>
</div>
```

### 2.4 CSS 改动

`DashboardDesignerView.module.css` 中：

- `telemetryInsertRow` 已存在（用于 dynamic properties 表达式弹窗），确认其 `display: flex` + `gap` 适合两个按钮并排、Format+Preview 并排两种场景；如按钮间距与字段间距视觉差异大，新增 `telemetryButtonRow` / `telemetryFieldRow` 两个变体类
- 不新增额外 CSS 命名空间，复用现有 `field` / `input` / `select` / `button` / `primary` / `modalActions` / `fieldError` 类

---

## 三、函数选择弹窗

### 3.1 新增 state

在 `DashboardDesignerView.tsx` 第 1411 行附近的 state 区块新增：

```tsx
const [functionPickerOpen, setFunctionPickerOpen] = useState(false);
const [pickerFunctionName, setPickerFunctionName] = useState<string | null>(null);
```

### 3.2 内置函数元数据

在文件顶部（`TEXT_FORMAT_OPTIONS` 常量附近，约第 82 行）新增函数元数据表，供弹窗展示与插入模板共用：

```tsx
interface BuiltinFunctionMeta {
  name: string;
  signature: string;
  description: string;
  /** 插入模板，| 代表光标停留位置（选区覆盖） */
  template: string;
}

const BUILTIN_FUNCTIONS: BuiltinFunctionMeta[] = [
  {
    name: "abs",
    signature: "abs(x)",
    description: "取绝对值",
    template: "abs(|)",
  },
  {
    name: "round",
    signature: "round(x, n)",
    description: "四舍五入保留 n 位小数",
    template: "round(|, 2)",
  },
];
```

> 当函数规范新增函数时，本表与 `acc-coach` 设计时求值器的函数实现表（见第四节）必须同步更新。

### 3.3 弹窗 JSX

参考现有 `telemetryPickerOpen` 弹窗（第 3331-3388 行）的结构，在 `textContentOpen` 弹窗之后追加：

```tsx
{functionPickerOpen && (
  <div className={styles.modalOverlay}>
    <div className={styles.modalSmall}>
      <div className={styles.modalTitle}>Insert Function</div>
      <div className={styles.telemetryList}>
        {BUILTIN_FUNCTIONS.map((fn) => (
          <button
            key={fn.name}
            className={`${styles.telemetryItem} ${
              pickerFunctionName === fn.name ? styles.telemetryItemActive : ""
            }`}
            type="button"
            onClick={() => setPickerFunctionName(fn.name)}
            onDoubleClick={() => {
              insertFunctionToken(fn);
              setFunctionPickerOpen(false);
            }}
          >
            <span>{fn.signature}</span>
            <span>{fn.description}</span>
          </button>
        ))}
      </div>
      <div className={styles.modalActions}>
        <div className={styles.modalActionSpacer} />
        <button
          className={styles.primary}
          type="button"
          onClick={() => {
            const fn = BUILTIN_FUNCTIONS.find(
              (f) => f.name === pickerFunctionName,
            );
            if (fn) insertFunctionToken(fn);
            setFunctionPickerOpen(false);
          }}
          disabled={!pickerFunctionName}
        >
          OK
        </button>
        <button
          className={styles.button}
          type="button"
          onClick={() => setFunctionPickerOpen(false)}
        >
          Cancel
        </button>
      </div>
    </div>
  </div>
)}
```

### 3.4 openFunctionPicker

参考 `openTelemetryPicker`（第 1796 行）的实现，记录当前文本框选区位置：

```tsx
const openFunctionPicker = () => {
  const textarea = textDraftRef.current;
  const start = textarea?.selectionStart ?? textDraft.length;
  const end = textarea?.selectionEnd ?? start;
  textInsertRangeRef.current = { start, end };
  setPickerFunctionName(null);
  setFunctionPickerOpen(true);
};
```

### 3.5 insertFunctionToken

参考 `insertTelemetryToken`（第 1834 行）的实现，插入函数模板并把光标定位到 `|` 占位符位置：

```tsx
const insertFunctionToken = (fn: BuiltinFunctionMeta) => {
  const textarea = textDraftRef.current;
  const insertRange = textInsertRangeRef.current;
  const start = insertRange?.start ?? textarea?.selectionStart ?? textDraft.length;
  const end = insertRange?.end ?? textarea?.selectionEnd ?? start;

  const replacesTemplate = canReplaceTextTemplate(textDraft, channels);
  const template = fn.template;
  const placeholderIndex = template.indexOf("|");
  if (placeholderIndex < 0) {
    // 无占位符的模板：直接插入，光标在末尾
    const cursor = replacesTemplate ? template.length : start + template.length;
    setTextDraft((current) => {
      if (canReplaceTextTemplate(current, channels)) return template;
      return `${current.slice(0, start)}${template}${current.slice(end)}`;
    });
    textInsertRangeRef.current = { start: cursor, end: cursor };
    window.requestAnimationFrame(() => {
      textDraftRef.current?.focus();
      textDraftRef.current?.setSelectionRange(cursor, cursor);
    });
    return;
  }

  // 有占位符：插入后选中 | 位置
  const insertStart = replacesTemplate ? 0 : start;
  const insertEnd = replacesTemplate ? 0 : end;
  const cursorStart = insertStart + placeholderIndex;
  const cursorEnd = cursorStart + 1; // 选中 "|" 一个字符
  setTextDraft((current) => {
    if (canReplaceTextTemplate(current, channels))
      return template.replace("|", "");
    return `${current.slice(0, start)}${template}${current.slice(end)}`.replace("|", "");
  });
  // 注意：上面把 | 从模板中移除，光标定位到原本 | 的位置
  textInsertRangeRef.current = { start: cursorStart, end: cursorStart };
  window.requestAnimationFrame(() => {
    textDraftRef.current?.focus();
    textDraftRef.current?.setSelectionRange(cursorStart, cursorStart);
  });
};
```

> **占位符 `|` 的处理**：`|` 不是表达式合法 token，若保留会导致预览报错。因此插入时**移除 `|`**，光标定位到 `|` 原位置。用户紧接着点 "insert telemetry item" 时，`insertTelemetryToken` 会用 `{field}` token 替换当前选区/光标位置（已有逻辑支持，见第 1834-1854 行）。

> **实现注意**：上述代码片段中 `template.replace("|", "")` 只会移除第一个 `|`，与本场景一致（每个模板至多一个 `|`）。若未来引入多参数模板需要多 `|`，需重新设计占位符机制。

### 3.6 与现有 insertTelemetryToken 的协作

用户插入函数后，`|` 被移除，光标停在参数位置。用户接着点 "insert telemetry item"：

1. `openTelemetryPicker` 读取 `textDraftRef.current.selectionStart`（即 `|` 原位置）写入 `textInsertRangeRef`
2. 用户选择 channel，`chooseTelemetryItem` → `insertTelemetryToken` 在光标位置插入 `{field}` token
3. 文本变为 `round({speedKmh}, 2)`

此流程无需改动 `insertTelemetryToken` / `chooseTelemetryItem` 的现有逻辑。

---

## 四、设计时求值器扩展

### 4.1 当前实现位置

`DashboardDesignerView.tsx` 第 519-862 行：

- `tokenizeExpression`（第 519 行）— 已支持把任意标识符保留为 `identifier` token（通过 `knownIdentifiers` 匹配 + 字母兜底分支第 588-602 行），**无需改动**
- `ExpressionParser` 类（第 660-852 行）— `parsePrimary`（第 788-810 行）不支持函数调用，**需扩展**
- `RESERVED_EXPRESSION_WORDS`（第 96 行）— `validateExpression` 用其判定"合法非字段标识符"，**需加入函数名**（或新增 `BUILTIN_FUNCTION_NAMES` 集合）

### 4.2 parsePrimary 扩展

当前 `parsePrimary`（第 788-810 行）：

```tsx
private parsePrimary(): ExpressionValue {
  const token = this.advance();
  if (!token) throw new Error("Unexpected end of expression");
  if (token.type === "number" || token.type === "string") return token.value;
  if (token.type === "identifier") {
    if (token.value === "true") return true;
    if (token.value === "false") return false;
    if (token.value === "null") return null;
    if (token.value === "undefined") return undefined;
    if (token.value === "NaN") return NaN;
    if (token.value === "Infinity") return Infinity;
    if (Object.prototype.hasOwnProperty.call(this.context, token.value)) {
      return this.context[token.value];
    }
    throw new Error(`Unknown telemetry field: ${token.value}`);
  }
  if (token.type === "paren" && token.value === "(") {
    const value = this.parseConditional();
    this.consumeRightParen();
    return value;
  }
  throw new Error("Unexpected token");
}
```

目标实现：

```tsx
private parsePrimary(): ExpressionValue {
  const token = this.advance();
  if (!token) throw new Error("Unexpected end of expression");
  if (token.type === "number" || token.type === "string") return token.value;
  if (token.type === "identifier") {
    if (token.value === "true") return true;
    if (token.value === "false") return false;
    if (token.value === "null") return null;
    if (token.value === "undefined") return undefined;
    if (token.value === "NaN") return NaN;
    if (token.value === "Infinity") return Infinity;

    // 函数调用：identifier 后紧跟 "("
    const next = this.peek();
    if (next?.type === "paren" && next.value === "(") {
      this.advance(); // 消耗 "("
      const args: ExpressionValue[] = [];
      const afterOpen = this.peek();
      if (afterOpen?.type === "paren" && afterOpen.value === ")") {
        this.advance();
      } else {
        while (true) {
          args.push(this.parseConditional());
          const sep = this.advance();
          if (sep?.type === "paren" && sep.value === ")") break;
          if (sep?.type !== "operator" || sep.value !== ",")
            throw new Error("Expected ',' or ')' in function arguments");
        }
      }
      const fn = EVALUATOR_BUILTIN_FUNCTIONS[token.value];
      if (!fn) throw new Error(`Unknown function: ${token.value}`);
      return fn(args);
    }

    // 字段引用
    if (Object.prototype.hasOwnProperty.call(this.context, token.value)) {
      return this.context[token.value];
    }
    throw new Error(`Unknown telemetry field: ${token.value}`);
  }
  if (token.type === "paren" && token.value === "(") {
    const value = this.parseConditional();
    this.consumeRightParen();
    return value;
  }
  throw new Error("Unexpected token");
}
```

### 4.3 求值器内置函数表

在 `ExpressionParser` 类之前（建议放在 `evaluateSafeExpression` 函数附近，约第 854 行前）新增：

```tsx
type ExpressionValue = string | number | boolean | null | undefined;

const EVALUATOR_BUILTIN_FUNCTIONS: Record<
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

> 此表必须与函数规范第 4.3 节、`module_local_dashboard` 的 `BUILTIN_FUNCTIONS` 完全一致。错误信息文本必须一字不差，便于跨模块交叉比对。

### 4.4 RESERVED_EXPRESSION_WORDS 调整

当前 `RESERVED_EXPRESSION_WORDS`（第 96-109 行）：

```tsx
const RESERVED_EXPRESSION_WORDS = new Set([
  "true", "false", "null", "undefined", "NaN", "Infinity",
  "Math", "Number", "String", "Boolean", "parseFloat", "parseInt",
]);
```

`validateExpression`（第 1195-1203 行）用它判定"未知标识符"：

```tsx
const unknown = expressionIdentifiers(
  prepared.expression,
  Object.keys(prepared.context),
).filter(
  (name) =>
    !Object.prototype.hasOwnProperty.call(prepared.context, name) &&
    !RESERVED_EXPRESSION_WORDS.has(name),
);
if (unknown.length > 0) return `Unknown telemetry field: ${unknown[0]}`;
```

若不调整，`round` / `abs` 会被 `validateExpression` 判定为 `Unknown telemetry field`，导致 dynamic properties 的表达式编辑器（visibleExpression 等）拒绝函数调用。

**调整方案**：新增独立集合，避免污染 `RESERVED_EXPRESSION_WORDS`（后者还含 `Math`/`Number` 等运行时全局，语义不同）：

```tsx
const BUILTIN_FUNCTION_NAMES = new Set(["abs", "round"]);
```

`validateExpression` 的过滤条件改为：

```tsx
const unknown = expressionIdentifiers(
  prepared.expression,
  Object.keys(prepared.context),
).filter(
  (name) =>
    !Object.prototype.hasOwnProperty.call(prepared.context, name) &&
    !RESERVED_EXPRESSION_WORDS.has(name) &&
    !BUILTIN_FUNCTION_NAMES.has(name),
);
```

> **注意**：`validateExpression` 用于 dynamic properties 的 `visibleExpression` / `textColorExpression` / `backgroundColorExpression` 等表达式编辑器。本方案的 UI 改动（insert function 按钮）仅作用于 text widget 的 Text Content 弹窗，但**求值器扩展对 dynamic properties 表达式同样生效**——这是合理的副产：用户在 visibleExpression 中写 `abs({latG}) > 1.5` 也能工作。若产品上不希望 dynamic properties 支持函数，需在 `validateExpression` 中额外限定；当前方案默认允许。

### 4.5 与 text widget 求值的关系

`evaluateTextExpression`（第 864 行）调用 `evaluateSafeExpression`（第 854 行），后者构造 `ExpressionParser`。第 4.2 节对 `parsePrimary` 的扩展自动对 text widget 表达式生效，无需额外改动 `evaluateTextExpression`。

---

## 五、改动文件清单

| 文件 | 改动类型 | 说明 |
|------|----------|------|
| `src-ui/components/DashboardDesignerView.tsx` | 修改 | UI 重构（第 3249-3310 行）、新增函数选择弹窗、新增 state、新增 `openFunctionPicker` / `insertFunctionToken` / `BUILTIN_FUNCTIONS` 元数据表、扩展 `parsePrimary`、新增 `EVALUATOR_BUILTIN_FUNCTIONS`、新增 `BUILTIN_FUNCTION_NAMES` 并接入 `validateExpression` |
| `src-ui/components/DashboardDesignerView.module.css` | 可能修改 | 若 `telemetryInsertRow` 在"两按钮并排"与"Format+Preview 并排"两种场景下视觉差异大，新增变体类 |
| `src-ui/components/DashboardDesignerView.test.tsx` | 修改 | 新增函数插入与求值的单元测试 |

---

## 六、验收标准

### 6.1 UI

1. Text Content 弹窗为 5 行布局，第 2 行仅含 "Insert function" 与 "Insert telemetry item" 两个按钮
2. "Example raw value" 独占第 3 行
3. "Format" 下拉框与 "Preview" 文本框在第 4 行并排
4. OK / Cancel 在第 5 行
5. 点 "Insert function" 弹出函数选择列表，显示 `abs(x)` / `round(x, n)` 及说明
6. 选 `abs` 插入后文本框含 `abs()`，光标在括号内
7. 选 `round` 插入后文本框含 `round(, 2)`，光标在第一个逗号前的空参数位置
8. 插入函数后点 "Insert telemetry item" 选择字段，字段 token 插入到光标位置

### 6.2 设计时求值

9. `evaluateTextExpression("round({speedKmh}, 2)", ...)` 在 `{speedKmh}`=164.389 时返回 `"164.39"`
10. `evaluateTextExpression("abs({latG})", ...)` 在 `{latG}`=-1.3 时返回 `"1.3"`
11. 函数嵌套 `round(abs({latG}), 2)` 求值正确
12. 未知函数抛 `Unknown function: <name>`，错误信息显示在 Preview 下方
13. 参数个数错误抛 `Invalid arguments for <fn>`
14. 函数规范第六节全部 17 个测试用例通过

### 6.3 兼容性

15. 旧 text widget（不含函数调用）的预览与保存行为不变
16. `validateExpression` 对 dynamic properties 表达式仍正常工作；含函数的表达式也能通过校验
17. `DashboardDesignerView.test.tsx` 现有用例不回归

---

## 七、测试计划

`DashboardDesignerView.test.tsx` 新增用例：

- 函数选择弹窗打开/关闭
- 插入 `abs` / `round` 后 `textDraft` 的内容与光标位置
- 插入函数后插入 telemetry token 的协作流程
- `evaluateTextExpression` 对函数调用的求值（覆盖函数规范第六节用例）
- `validateExpression` 接受含函数的表达式

测试中 `channelById` 与 `exampleValues` 的构造参考现有 `DashboardDesignerView.test.tsx` 第 34 行附近的 fixture。

---

## 八、与 module_local_dashboard 的协同上线

- 本方案（acc-coach UI + 设计时求值器）与 `docs/acc-coach/prd/module-local-dashboard-text-expression-functions.md`（module_local_dashboard 运行时求值器）建议**同批次发布**
- 若 acc-coach 先发布：用户插入的函数在设计时预览正确，但 overlay 运行时显示空串（求值失败被 catch 吞掉）。不崩溃但体验不佳
- 若 module_local_dashboard 先发布：无负面影响（旧 text widget 不含函数调用，新求值器向后兼容）
- 两边上线后，使用含函数调用的同一份 layout 在 Dashboard Designer 预览与 overlay 实际渲染中对比验证一致性

---

## 九、未来扩展

- 新增内置函数：更新函数规范第三节 → 同步更新本文档第三节元数据表 + 第四节求值器函数表 + `module_local_dashboard` PRD 的函数表
- 函数选择弹窗支持搜索：当函数数量增多时，在弹窗顶部加搜索框（参考 telemetry picker 的 `telemetrySearch`）
- 函数签名提示：在弹窗中选中函数时显示参数说明（当前仅展示 signature + description）
- 多占位符模板：当前模板仅支持单个 `|` 占位符；若未来引入多参数模板（如 `substr(|, |, |)`），需重新设计占位符机制（例如用 `{{1}}` / `{{2}}` 编号占位 + Tab 跳转）
