---
marp: true
theme: adam-slides
size: 16:9
paginate: true
title: Adam - Live Tutorial
description: Interactive examples of the Adam property-model language.
---

# About Adam

- Adam is a language to describe the relationships between values
- The basic construct is a sheet containing cells and relationships between cells
- As cells are edited, the relationships are upheld in a predictable manner
  - The primary use is to control UI behavior
- Sheets support recording and playback, designed to support Ps Actions
  - Capturing a dictionary (unordered) of the source values and applying it to a fresh sheet guarantees the same result

---

# A first sheet

Change `width` or `height`. Source cells hold the values you write.

<!-- adam-example: tutorial/first_sheet graph=1 -->

---

# Filters

Enter a value outside 0-100 and watch `level` return to its domain.

<!-- adam-example: tutorial/clamp_demo -->

---

# Out cells

Change `width` or `height`. The forced output `area` follows automatically.

<!-- adam-example: tutorial/basic_output graph=1 -->

---

# Cells and relationships

Edit either `a` or `b`. Watch the graph reverse its flow to preserve your edit.

<!-- adam-example: tutorial/basic_relationship graph=1 -->

---

# Chained relationships

Move `a` to 100, then back to 0. The previous values of `b` and `c` return.

<!-- adam-example: tutorial/inequality graph=1 -->

---

# Conditionals

Enable `constrain` to link `a` and `b`; disable it to edit them independently.

<!-- adam-example: tutorial/constrain graph=1 -->

---

# Forced conditional values

Enable `constrain` to force `a` to 42. Disable it to restore the source value.

When the value of a cell is forced, the widget is disabled.

<!-- adam-example: tutorial/conditional_forced -->

---

# Filter diagnostics

The derived `width` exceeds its filter. Edit `height` to explore the diagnostic.

<!-- adam-example: tutorial/requirements_filter_diagnostic -->

---

# Requirements

Set `width` and `height` to 100. The output reports its violated requirement.

<!-- adam-example: tutorial/area_with_requirement -->
