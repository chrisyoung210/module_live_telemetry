# Text Widget 表达式内置函数调用规范

> 适用模块：`acc-coach`（设计时预览求值器）、`module_local_dashboard`（运行时渲染求值器）
>
> 文档类型：交互协议 / 双方一致性契约
>
> 状态：等待开发

---

## 一、背景

ACC Coach 的 text widget 支持在 `textTemplate` 中嵌入 `{{expr:...}}` 表达式，对 telemetry 值做运算后输出。表达式由两个独立实现的中缀表达式求值器负责解析：

| 求值器 | 位置 | 用途 | 触发时机 |
|--------|------|------|----------|
| 设计时预览 | `acc-coach` `src-ui/components/DashboardDesignerView.tsx` (`ExpressionParser`) | 在 Dashboard Designer 编辑文本模板时实时计算 Preview 值 | 用户编辑 text widget 弹窗 |
| 运行时渲染 | `module_local_dashboard` `src-ui/features/local-dashboard-overlay/textExpression.ts` (`Parser`) | overlay 实际显示时计算每个 text widget 的最终文本 | overlay 每帧渲染 |

两个求值器当前都支持：字面量、`{field}` 占位符、四则运算、比较、逻辑、三元运算。**都不支持函数调用**——遇到 `round`、`abs` 这类标识符时直接抛 `Unknown identifier` / `Unknown telemetry field`。

本规范定义 text widget 表达式中的**内置函数调用语法**，作为两个求值器的一致性契约。两个实现必须语义完全一致，否则设计时预览与运行时渲染会出现偏差。

---

## 二、语法

### 2.1 函数调用形式

```
identifier ( argument , argument , ... )
```

- `identifier` 必须是本规范第三节定义的**内置函数名**之一
- 参数列表由逗号 `,` 分隔
- 参数本身可以是任意合法子表达式（字面量、占位符、运算式、嵌套函数调用）
- 参数个数必须与函数签名匹配，否则抛 `Invalid arguments for <fn>` 错误
- 函数调用可以出现在任何"值"可出现的位置（一元/二元/三元运算的操作数、比较运算的操作数、函数参数等）
- 函数调用可以嵌套：`round(abs({field}), 2)`

### 2.2 与 `{field}` 占位符的求值顺序

`{field}` 占位符在表达式求值**之前**被替换为字面量：

```
round({speedKmh}, 2)
  ↓ 占位符替换（示例值 164.389）
round(164.389, 2)
  ↓ 函数求值
164.39
```

因此函数参数中的 `{field}` 总是被替换为一个 **JSON 字面量**（数字、字符串、布尔等），不是变量引用。函数实现接收的是已求值的字面量值。

### 2.3 表达式存储形式

含函数调用的表达式仍存储在 `textTemplate` 字段中，外层包裹 `{{expr:...}}`：

```
textTemplate = "{{expr:round({speedKmh}, 2) + \" km/h\"}}"
```

这与现有表达式存储形式一致，不引入新的存储结构。

### 2.4 大小写

函数名**大小写敏感**，必须小写。`Round(...)`、`ABS(...)` 视为未知标识符并抛错。

---

## 三、内置函数表

本规范定义的内置函数集合由 acc-coach 与 module_local_dashboard **共同实现、共同维护**。新增函数需更新本规范并同时修改两个求值器。

> 设计原则：**仅内置，不提供 UDF**。用户不能自定义函数，只能使用本表列出的函数。

### 3.1 `abs(x)`

| 项 | 值 |
|----|----|
| 签名 | `abs(x)` |
| 参数 | `x` — number（或可强制转换为 number 的值） |
| 返回 | number |
| 语义 | 返回 `x` 的绝对值，等价 `Math.abs(x)` |
| 示例 | `abs(-3.5)` → `3.5`；`abs({latG})` → 始终非负的横向 G 值 |

### 3.2 `round(x, n)`

| 项 | 值 |
|----|----|
| 签名 | `round(x, n)` |
| 参数 | `x` — number；`n` — 非负整数，表示保留的**小数位数** |
| 返回 | number |
| 语义 | 对 `x` 做四舍五入，保留 `n` 位小数，等价 `Number(x.toFixed(n))` |
| `n = 0` | `round(3.7, 0)` → `4` |
| `n = 2` | `round(3.14159, 2)` → `3.14` |
| `n` 为非整数 | 向下取整后使用（`round(x, 2.9)` 等同 `round(x, 2)`） |
| `n` 为负数 | 抛 `Invalid arguments for round: n must be >= 0` |
| `x` 为非有限数 (NaN/Infinity) | 原样返回（`round(NaN, 2)` → `NaN`） |
| 示例 | `round({speedKmh}, 1)` → 车速保留 1 位小数 |

> **注意**：`round` 的 `n` 是**小数位数**，不是"有效数字位数"。`round(1234, 2)` = `1234`，不是 `1200`。

### 3.3 未来扩展

新增内置函数时：
1. 必须先在本规范第三节添加函数条目（签名、参数、返回、语义、边界行为）
2. acc-coach 与 module_local_dashboard **同时**实现该函数，语义以本规范为准
3. 不引入任何"用户自定义函数"机制

---

## 四、求值器契约

两个求值器实现必须满足以下共同契约。

### 4.1 tokenizer 行为

遇到以字母/下划线/`$` 开头的标识符时：

1. 读取完整标识符（`[A-Za-z_$][A-Za-z0-9_$]*`）
2. 若是保留字（`true` / `false` / `null` / `undefined` / `NaN` / `Infinity`）→ 产出对应字面量 token
3. **否则产出 `identifier` token，保留标识符文本，不在此阶段抛错**
4. 是否为合法函数名 / 合法字段名，由 parser 在遇到 `(` 或读取结束时判定

> **关键修正点**：`module_local_dashboard` 的 `textExpression.ts` 当前在 tokenizer 阶段（第 75-93 行）对非保留字标识符**直接抛 `Unknown identifier`**，导致 `round` / `abs` 永远无法进入 parser。必须改为保留为 `identifier` token。`acc-coach` 的 `tokenizeExpression`（`DashboardDesignerView.tsx:519`）已通过 `knownIdentifiers` 匹配保留标识符 token，无需改动 tokenizer。

### 4.2 parser 行为

在解析"主表达式"（primary / unary 层）时，遇到 `identifier` token：

1. **预读下一个 token**
2. 若下一个 token 是 `(` → 解析为**函数调用**：
   - 消耗 `(` token
   - 解析参数列表：零个或多个由 `,` 分隔的子表达式（每个子表达式用 `parseConditional` / 同级入口解析）
   - 消耗 `)` token
   - 在内置函数表中查找该函数名；未找到 → 抛 `Unknown function: <name>`
   - 校验参数个数；不匹配 → 抛 `Invalid arguments for <fn>`
   - 执行函数实现，返回值作为该 primary 的求值结果
3. 若下一个 token 不是 `(` → 按现有规则处理为**变量引用**（查 context / 抛 `Unknown telemetry field`）

### 4.3 内置函数实现表

两个求值器必须维护同一张函数表，实现语义一致：

```typescript
const BUILTIN_FUNCTIONS = {
  abs: (args: ExpressionValue[]) => {
    if (args.length !== 1) throw new Error("Invalid arguments for abs");
    return Math.abs(Number(args[0]));
  },
  round: (args: ExpressionValue[]) => {
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

> 各实现可按自身语言/代码风格调整，但**输入校验、错误信息、返回值**必须与上表等价。

### 4.4 错误处理契约

| 情况 | 错误信息 | 抛出位置 |
|------|----------|----------|
| 未知函数名 | `Unknown function: <name>` | parser |
| 参数个数不匹配 | `Invalid arguments for <fn>` | 函数实现 |
| `round` 的 `n` 为负数或非有限数 | `Invalid arguments for round: n must be >= 0` | 函数实现 |
| 未知标识符（非函数、非字段） | `Unknown telemetry field: <name>` | parser |

> 设计时求值器（acc-coach）的错误信息会显示在弹窗 Preview 下方的错误区；运行时求值器（module_local_dashboard）的错误会被 `resolveControlText` 的 `catch` 吞掉并返回空串（现有行为，不需改动）。**但两边抛出的错误信息文本必须一致**，便于设计时定位问题。

---

## 五、占位符替换与函数调用的交互

`{field}` 占位符替换为字面量的规则与现有表达式一致，不因函数调用而改变：

| 占位符形式 | 替换为 | 说明 |
|-----------|--------|------|
| `{field}` | `JSON.stringify(rawValue)` | 字符串字面量；数字字段也先 stringify 再被表达式 parser 解析为 number |
| `{field\|format}` | `JSON.stringify(formattedValue)` | 先按 format 格式化，再 stringify |
| `{value}` | 取 `control.telemetryField` 对应的值 | `value` 是当前控件绑定字段的别名 |

**函数参数中的占位符同样遵循此规则**：

```
round({speedKmh|0}, 2)
  ↓ {speedKmh|0} 先格式化为 "164"（整数）
round(164, 2)
  ↓
164
```

> 这是一个值得注意的边界情况：先 format 再 round 可能不是用户预期。文档/UI 提示应建议直接用 `round({speedKmh}, 2)` 而非 `round({speedKmh|0}, 2)`。

---

## 六、一致性测试用例

以下用例必须两个求值器都能正确求值，且结果一致。各实现应将其纳入单元测试。

| # | 表达式（占位符已替换后） | 期望结果 | 备注 |
|---|--------------------------|----------|------|
| 1 | `abs(-3.5)` | `3.5` | 基本绝对值 |
| 2 | `abs(0)` | `0` | 零值 |
| 3 | `abs(3.5)` | `3.5` | 正数不变 |
| 4 | `round(3.14159, 2)` | `3.14` | 两位小数 |
| 5 | `round(3.14159, 0)` | `3` | 零位小数 |
| 6 | `round(3.5, 0)` | `4` | 四舍五入进位 |
| 7 | `round(3.4, 0)` | `3` | 四舍五入舍去 |
| 8 | `round(1234, 2)` | `1234` | 整数不受影响 |
| 9 | `round(2.5, 0)` | `3`（注意 JS toFixed 行为） | 与 `Number((2.5).toFixed(0))` 一致 |
| 10 | `round(abs(-3.14159), 2)` | `3.14` | 嵌套调用 |
| 11 | `abs(round(-3.6, 0))` | `4` | 反向嵌套 |
| 12 | `round({speedKmh}, 1) + " km/h"`（{speedKmh}=164.389） | `"164.4 km/h"` | 与字符串拼接 |
| 13 | `abs({latG}) > 1.5 ? "HIGH" : "ok"` | 条件判断中使用函数 | |
| 14 | `round({speedKmh}, 2)` （{speedKmh}=NaN） | `NaN` | 非有限值原样返回 |
| 15 | `unknownFn(1)` | 抛 `Unknown function: unknownFn` | 未知函数 |
| 16 | `abs(1, 2)` | 抛 `Invalid arguments for abs` | 参数个数错误 |
| 17 | `round(1, -1)` | 抛 `Invalid arguments for round: n must be >= 0` | n 为负 |

---

## 七、不属于本规范的内容

- 不定义 UDF（用户自定义函数）机制
- 不定义函数副作用（所有内置函数必须是纯函数）
- 不定义函数对非 number 参数的强制转换规则（除 `Number()` 隐式转换外不做额外处理）
- 不修改 `DashboardControl` 协议类型（函数调用是 `textTemplate` 字符串内部语法，不影响协议字段）
- 不涉及 `module_dashboard_protocol` 的任何改动

---

## 八、验收标准

1. `acc-coach` 设计时求值器支持本规范第三节全部函数，通过第六节测试用例
2. `module_local_dashboard` 运行时求值器支持本规范第三节全部函数，通过第六节测试用例
3. 两个求值器对同一表达式求值结果完全一致（包括错误信息文本）
4. `textTemplate` 存储格式不变，旧布局（不含函数调用）继续正常工作
5. 未来新增函数时，本规范第三节是唯一的真理源
