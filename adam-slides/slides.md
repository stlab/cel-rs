---
marp: true
theme: adam-slides
size: 16:9
paginate: true
title: Adam - Live Tutorial
description: Nine interactive examples of the Adam property-model language.
---

# A first sheet

Change `width` or `height`. Source cells hold the values you write.

<!-- adam-example: tutorial/first_sheet -->

---

# Filters

Enter a value outside 0-100 and watch `level` return to its domain.

<!-- adam-example: tutorial/clamp_demo -->

---

# Out cells

Change `width` or `height`. The forced output `area` follows automatically.

<!-- adam-example: tutorial/basic_output -->

---

# Cells and relationships

Edit either `a` or `b`. Watch the graph reverse its flow to preserve your edit.

<!-- adam-example: tutorial/basic_relationship -->

---

# Chained relationships

Move `a` to 100, then back to 0. The previous values of `b` and `c` return.

<!-- adam-example: tutorial/inequality -->

---

# Conditionals

Enable `constrain` to link `a` and `b`; disable it to edit them independently.

<!-- adam-example: tutorial/constrain -->

---

# Forced conditional values

Enable `constrain` to force `a` to 42. Disable it to restore the source value.

<!-- adam-example: tutorial/conditional_forced -->

---

# Filter diagnostics

The derived `width` exceeds its filter. Edit `height` to explore the diagnostic.

<!-- adam-example: tutorial/requirements_filter_diagnostic -->

---

# Requirements

Set `width` and `height` to 100. The output reports its violated requirement.

<!-- adam-example: tutorial/area_with_requirement -->
