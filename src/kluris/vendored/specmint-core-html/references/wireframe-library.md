# Wireframe Library

Ready-to-use wireframe patterns built from the `.wf-*` primitives in `spec-styles.css`. Copy a pattern into a `<figure class="mockup mockup--wireframe">` and customize.

Wireframes communicate **structure and hierarchy** without committing to visual design. Grayscale boxes, dashed canvas border, optional annotation callouts with SVG arrows.

> ## ⚠️ Hard rule — empty bars only
>
> `.wf-heading`, `.wf-text`, `.wf-pill`, `.wf-input` are **empty skeleton bars**. They render as grayscale rectangles when the tag is empty:
>
> ```html
> <span class="wf-heading"></span>     ✅ grey heading bar
> <span class="wf-text"></span>         ✅ grey body-text bar
> ```
>
> The moment you have real text to show, **you are authoring hi-fi**. Switch the figure to `mockup--hifi` and use the `.ui-*` library (`references/mockup-library.md`) instead:
>
> ```html
> <span class="wf-heading">Sites</span>          ❌ ugly grey blob behind "Sites"
> <h2 class="ui-card__title">Sites</h2>          ✅ real heading
>
> <span class="wf-text">AERE1</span>             ❌ grey strip behind real text
> <strong>AERE1</strong>                          ✅ real text
>
> <span class="wf-pill"></span>                   ✅ empty status placeholder
> <span class="ui-tag ui-tag--success">active</span>  ✅ real status badge
> ```
>
> Mixing real text inside `.wf-*` primitives is the #1 cause of "ugly grey blob behind text" bug reports.

## Primitives reference

| Class | Renders as |
|-------|------------|
| `.wf-frame` | Dashed-border canvas (the mockup container) |
| `.wf-row` / `.wf-col` | Flex row / column |
| `.wf-row--header` | Row with bottom border, header treatment |
| `.wf-card` | Light gray rounded box |
| `.wf-sidebar` | Fixed-width sidebar column |
| `.wf-content` | Flex-1 main content area |
| `.wf-content--center` | Centered flex column |
| `.wf-heading` / `.wf-heading--sm` / `.wf-heading--lg` | Heading bar |
| `.wf-text` (multiple) | Body text placeholder lines (auto-varying widths) |
| `.wf-input` | Input rectangle |
| `.wf-input--label` | Label text above input |
| `.wf-button` / `.wf-button--primary` / `.wf-button--ghost` | Button variants |
| `.wf-icon` / `.wf-icon--lg` / `.wf-icon--square` | Icon placeholder |
| `.wf-pill` | Small pill placeholder |
| `.wf-divider` | Horizontal rule |
| `.wf-list-item` / `.wf-list-item--active` | Sidebar/list row |
| `.wf-image` | Image placeholder (diagonal hatching) |
| `.wf-table` (use `style="--cols: 5"`) | Grayscale table grid |
| `.wf-spacer` / `.wf-spacer--lg` | Vertical spacing |
| `.wf-annotation` | Labeled callout (combine with `data-points-to="ID"` and `.wf-annotation--top-right` etc.) |

## Patterns

### App shell

```html
<div class="wf-frame">
  <div class="wf-row wf-row--header">
    <span class="wf-heading wf-heading--lg"></span>
    <span class="wf-pill"></span>
  </div>
  <div class="wf-row wf-row--body" style="gap: var(--space-4);">
    <aside class="wf-sidebar">
      <div class="wf-list-item"></div>
      <div class="wf-list-item wf-list-item--active"></div>
      <div class="wf-list-item"></div>
    </aside>
    <main class="wf-content">
      <span class="wf-heading"></span>
      <span class="wf-text"></span>
      <span class="wf-text"></span>
    </main>
  </div>
</div>
```

### Form

```html
<div class="wf-frame">
  <span class="wf-heading"></span>
  <div class="wf-spacer"></div>
  <span class="wf-input--label">Email</span>
  <span class="wf-input" id="wf-form-email"></span>
  <span class="wf-input--label">Role</span>
  <span class="wf-input"></span>
  <div class="wf-spacer"></div>
  <span class="wf-button wf-button--primary" id="wf-form-cta">Send invite</span>
  <span class="wf-button wf-button--ghost" style="margin-left: 8px;">Cancel</span>
  <div class="wf-annotation wf-annotation--top-right" data-points-to="wf-form-email">Validates on blur</div>
</div>
```

### Empty state

```html
<div class="wf-frame">
  <div class="wf-content--center" style="padding: var(--space-8);">
    <span class="wf-icon wf-icon--lg"></span>
    <span class="wf-heading wf-heading--center"></span>
    <span class="wf-text"></span>
    <span class="wf-text"></span>
    <span class="wf-button wf-button--primary">Get started</span>
  </div>
</div>
```

### Table page (list view)

```html
<div class="wf-frame">
  <div class="wf-row wf-row--header">
    <span class="wf-heading"></span>
    <span class="wf-button wf-button--primary">+ Create</span>
  </div>
  <div class="wf-row" style="gap: var(--space-2);">
    <span class="wf-input" style="flex: 1;"></span>
    <span class="wf-button wf-button--ghost">Filter</span>
  </div>
  <div class="wf-spacer"></div>
  <div class="wf-table" style="--cols: 4;">
    <span class="wf-table__header"></span><span class="wf-table__header"></span><span class="wf-table__header"></span><span class="wf-table__header"></span>
    <span></span><span></span><span></span><span></span>
    <span></span><span></span><span></span><span></span>
    <span></span><span></span><span></span><span></span>
    <span></span><span></span><span></span><span></span>
  </div>
</div>
```

### Modal

```html
<div class="wf-frame" style="background: var(--neutral-bg); min-height: 240px; display: grid; place-items: center;">
  <div class="wf-card" style="width: 360px; background: #fff;">
    <span class="wf-heading"></span>
    <div class="wf-spacer"></div>
    <span class="wf-text"></span>
    <span class="wf-text"></span>
    <div class="wf-spacer"></div>
    <span class="wf-button wf-button--primary">Confirm</span>
    <span class="wf-button wf-button--ghost" style="margin-left: 8px;">Cancel</span>
  </div>
</div>
```

### Stepper / wizard

```html
<div class="wf-frame">
  <div class="wf-row" style="justify-content: center; gap: var(--space-3); margin-bottom: var(--space-4);">
    <span class="wf-pill"></span><span class="wf-pill"></span><span class="wf-pill"></span>
  </div>
  <span class="wf-heading wf-heading--center"></span>
  <div class="wf-spacer"></div>
  <span class="wf-text"></span><span class="wf-text"></span><span class="wf-text"></span>
  <div class="wf-spacer--lg"></div>
  <div class="wf-row" style="justify-content: space-between;">
    <span class="wf-button wf-button--ghost">Back</span>
    <span class="wf-button wf-button--primary">Next</span>
  </div>
</div>
```

### Detail / master

```html
<div class="wf-frame">
  <div class="wf-row" style="gap: var(--space-3); align-items: stretch;">
    <aside class="wf-sidebar" style="width: 240px;">
      <div class="wf-list-item wf-list-item--active"></div>
      <div class="wf-list-item"></div>
      <div class="wf-list-item"></div>
      <div class="wf-list-item"></div>
      <div class="wf-list-item"></div>
    </aside>
    <main class="wf-content">
      <span class="wf-heading"></span>
      <span class="wf-text"></span>
      <span class="wf-text"></span>
      <div class="wf-spacer"></div>
      <div class="wf-row" style="gap: var(--space-2);">
        <span class="wf-button wf-button--primary">Action</span>
        <span class="wf-button wf-button--ghost">Cancel</span>
      </div>
    </main>
  </div>
</div>
```

### Settings panel

```html
<div class="wf-frame">
  <span class="wf-heading wf-heading--lg"></span>
  <div class="wf-spacer"></div>
  <span class="wf-heading wf-heading--sm"></span>
  <span class="wf-input--label">Field 1</span>
  <span class="wf-input"></span>
  <span class="wf-input--label">Field 2</span>
  <span class="wf-input"></span>
  <div class="wf-divider"></div>
  <span class="wf-heading wf-heading--sm"></span>
  <span class="wf-input--label">Field 3</span>
  <span class="wf-input"></span>
  <div class="wf-spacer"></div>
  <span class="wf-button wf-button--primary">Save</span>
</div>
```

### Card grid

```html
<div class="wf-frame">
  <span class="wf-heading"></span>
  <div class="wf-spacer"></div>
  <div style="display: grid; grid-template-columns: repeat(3, 1fr); gap: var(--space-3);">
    <div class="wf-card"><span class="wf-image"></span><div class="wf-spacer"></div><span class="wf-heading wf-heading--sm"></span><span class="wf-text"></span></div>
    <div class="wf-card"><span class="wf-image"></span><div class="wf-spacer"></div><span class="wf-heading wf-heading--sm"></span><span class="wf-text"></span></div>
    <div class="wf-card"><span class="wf-image"></span><div class="wf-spacer"></div><span class="wf-heading wf-heading--sm"></span><span class="wf-text"></span></div>
  </div>
</div>
```

## Annotations

Position with one of `.wf-annotation--top-right`, `.wf-annotation--bottom-right`, `.wf-annotation--bottom-left`. Set `data-points-to="ID"` where ID is the `id` attribute of the target element inside the same `.wf-frame`. `spec-runtime.js` draws an SVG arrow at render time.

```html
<div class="wf-annotation wf-annotation--top-right" data-points-to="wf-cta">Disabled until form is valid</div>
```

Targets must have stable IDs:

```html
<span class="wf-button wf-button--primary" id="wf-cta">Send invite</span>
```

## When to use which

- **App shell** — first overview of where things live in the UI
- **Form / wizard** — input-heavy flows
- **Empty state** — zero-data screens (often needed for new features)
- **Table page** — list / index views
- **Modal** — confirmations, single-step inline tasks
- **Detail/master** — list + detail layouts (email, settings)
- **Settings panel** — grouped form sections
- **Card grid** — discovery / browse views

Compose patterns side-by-side in one `<figure>` to show before/after states or multi-screen flows.
