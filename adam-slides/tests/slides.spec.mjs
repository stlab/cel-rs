import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

const names = [
  "first_sheet", "clamp_demo", "basic_output", "basic_relationship",
  "inequality", "constrain", "conditional_forced",
  "requirements_filter_diagnostic", "area_with_requirement",
];

/** Returns the same-site host URL for one example and its optional graph. */
function host(name = "first_sheet", graph = false) {
  return `example.html?example=tutorial%2F${name}&graph=${graph ? "1" : "0"}`;
}

/** Catches missing live mounts, source drift, and runtime network dependencies. */
test("nine canonical examples mount without external runtime requests", async ({ page }, testInfo) => {
  const unexpected = [];
  page.on("pageerror", (error) => unexpected.push(error.message));
  page.on("request", (request) => {
    if (/^https?:/.test(request.url()) && new URL(request.url()).hostname !== "127.0.0.1") {
      unexpected.push(request.url());
    }
  });
  await page.goto("./");
  await expect(page.locator("iframe[data-example]")).toHaveCount(10);
  for (const name of names) {
    await selectSlide(page, name);
    const iframe = page.locator(`iframe[data-example="tutorial/${name}"]`);
    const frame = iframe.contentFrame();
    await expect(frame.locator(".adam-theme").first()).toBeAttached();
    await expect(frame.locator("#status")).toHaveCount(0);
    await expect(frame.getByRole("textbox").first()).toBeVisible();
    expect(await frame.locator("input, [role=slider], [role=checkbox]").count()).toBeGreaterThan(0);
    await expect(frame.locator("#stlab-adam-controls-v1")).toHaveCount(1);
    expect(await frame.getByRole("textbox").first().evaluate(element => getComputedStyle(element).borderRadius)).not.toBe("0px");
    const source = await readFile(new URL(`../../adam-lang-book/book-src/examples/tutorial/${name}.adm2`, import.meta.url), "utf8");
    const pane = iframe.locator("xpath=..");
    const rendered = await pane.locator("code.language-adam").textContent();
    expect(rendered.replace(/\r\n/g, "\n").trim()).toBe(source.replace(/\r\n/g, "\n").trim());
    expect((await pane.boundingBox()).width).toBeGreaterThan(1100);
    await page.screenshot({ path: testInfo.outputPath(`${name}.png`) });
  }
  await page.setViewportSize({ width: 960, height: 540 });
  await page.screenshot({ path: testInfo.outputPath("smaller-viewport.png") });
  expect(unexpected).toEqual([]);
});

/** Catches viewport-based graph sizing adding scrollbars to otherwise fitting examples. */
test("live panes fit their frames without unnecessary scrollbars", async ({ page }) => {
  await page.goto("./");
  const overflow = [];
  for (const width of [1280, 960]) {
    await page.setViewportSize({ width, height: width * 9 / 16 });
    for (const [index, name] of names.entries()) {
      const frame = await selectSlide(page, name);
      await expect(frame.locator("#status")).toHaveCount(0);
      if (await frame.locator("#graph").isVisible()) {
        await expect(frame.locator("#graph svg")).toBeVisible();
      }
      const size = await frame.locator("html").evaluate(async (element) => {
        await document.fonts.ready;
        return {
          width: element.clientWidth,
          height: element.clientHeight,
          scrollWidth: element.scrollWidth,
          scrollHeight: element.scrollHeight,
        };
      });
      if (size.scrollWidth > size.width || size.scrollHeight > size.height) {
        overflow.push({ name, viewport: width, ...size });
      }
    }
  }
  expect(overflow).toEqual([]);
});

/** Catches readonly numeric outputs truncating their formatted values in the shared inspector. */
test("area number field displays its entire formatted value", async ({ page }, testInfo) => {
  await page.goto("./");
  const frame = await selectSlide(page, "area_with_requirement");
  await writeSlider(frame, "width", 100);
  await writeSlider(frame, "height", 10);
  const input = frame.getByRole("textbox", {name:"area",exact:true});
  await expect(input).toHaveValue("1,000");
  const size = await input.evaluate(async (element) => {
    await document.fonts.ready;
    const style = getComputedStyle(element);
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d");
    context.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
    return {
      text: context.measureText(element.value).width,
      available: element.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight),
    };
  });
  expect(size.available).toBeGreaterThanOrEqual(size.text);
  await page.screenshot({ path: testInfo.outputPath("area-number-field.png") });
});

/** Catches slide numbers overlapping live panes instead of occupying the bottom margin. */
test("slide numbers stay below the example panes", async ({ page }, testInfo) => {
  await page.goto("./");
  for (const width of [1280, 960]) {
    await page.setViewportSize({ width, height: width * 9 / 16 });
    for (const [index, name] of names.entries()) {
      await selectSlide(page, name);
      const section = page.locator(`iframe[data-example="tutorial/${name}"]`)
        .locator("xpath=ancestor::section");
      const layout = await section.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const pane = element.querySelector("iframe").getBoundingClientRect();
        const number = getComputedStyle(element, "::after");
        const scale = bounds.height / element.clientHeight;
        const bottom = bounds.bottom - parseFloat(number.bottom) * scale;
        return { paneBottom: pane.bottom, numberTop: bottom - parseFloat(number.height) * scale, bottom, slideBottom: bounds.bottom };
      });
      expect(layout.numberTop).toBeGreaterThan(layout.paneBottom);
      expect(layout.bottom).toBeLessThan(layout.slideBottom);
      if (index === 2) await page.screenshot({ path: testInfo.outputPath(`out-cells-${width}.png`) });
    }
  }
});

/** Catches ambiguous URL options being silently accepted by the host. */
test("invalid example selectors display an alert", async ({ page }) => {
  await page.goto("example.html?example=..%2Fsecret&graph=0");
  await expect(page.getByRole("alert")).toContainText(/example/i);
});

/** Catches unknown manifest keys leaving an unexplained blank frame. */
test("unknown example displays an alert", async ({ page }) => {
  await page.goto(host("missing"));
  await expect(page.getByRole("alert")).toContainText(/missing|unknown|source/i);
});

for (const [name, file] of [
  ["manifest", "adam-live-examples.json"],
  ["module", "adam_lang_book_live.js"],
  ["WebAssembly", "adam_lang_book_live_bg.wasm"],
  ["graph", "graph.js"],
]) {
  /** Catches an unavailable runtime asset being mistaken for successful startup. */
  test(`${name} load failure displays an alert`, async ({ page }) => {
    await page.route(`**/${file}`, (route) => route.fulfill({ status: 404, body: "Missing asset" }));
    await page.goto(host("first_sheet", name === "graph"));
    await expect(page.getByRole("alert")).toContainText(/failed|error|404|fetch|load/i);
  });
}

/** Catches ignored WebAssembly initialization failures. */
test("WebAssembly initialization rejection displays an alert", async ({ page }) => {
  await page.route("**/adam_lang_book_live.js", (route) => route.fulfill({
    contentType: "text/javascript",
    body: 'export default async function init() { throw new Error("Wasm initialization failed"); } export function mount() {}',
  }));
  await page.goto(host());
  await expect(page.getByRole("alert")).toContainText("Wasm initialization failed");
});

for (const [name, file, graph, body] of [
  ["D3", "d3.v7.min.js", true, 'throw new Error("D3 execution failed");'],
  ["graph driver", "graph.js", true, 'throw new Error("Graph execution failed");'],
]) {
  /** Catches successfully fetched dependencies that cannot actually execute or register. */
  test(`${name} execution or registration failure displays an alert`, async ({ page }) => {
    await page.route(`**/${file}`, (route) => route.fulfill({ contentType: "text/javascript", body }));
    await page.goto(host("first_sheet", graph));
    await expect(page.getByRole("alert")).toContainText(/failed|missing|registered/i, { timeout: 2000 });
  });
}

/** Catches a missing bootstrap script preventing its own error boundary from running. */
test("host bootstrap load failure displays an alert", async ({ page }) => {
  await page.route("**/example.mjs", (route) => route.fulfill({ status: 404, body: "Missing bootstrap" }));
  await page.goto(host());
  await expect(page.getByRole("alert")).toContainText(/failed|fetch|load/i, { timeout: 2000 });
});

/** Catches graph dependencies loading after the sheet's first graph effect. */
test("graph dependencies finish loading before mount", async ({ page }) => {
  await page.route("**/adam_lang_book_live.js", (route) => route.fulfill({
    contentType: "text/javascript",
    body: `export default async function init() {}
      export function mount() {
        if (!window.d3 || !window.beginGraph) throw new Error("Dependencies not ready");
        document.getElementById("inspector").textContent = "Dependencies ready";
      }`,
  }));
  await page.goto(host("first_sheet", true));
  await expect(page.locator("#inspector")).toHaveText("Dependencies ready");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

/** Catches unnecessary graph dependencies on inspector-only examples. */
test("inspector-only examples do not load graph scripts", async ({ page }) => {
  const graphRequests = [];
  page.on("request", (request) => {
    if (/\/(swc\.js|graph\.js|d3\.v7\.min\.js)$/.test(request.url())) graphRequests.push(request.url());
  });
  await page.goto(host("clamp_demo"));
  await expect(page.getByRole("slider")).toBeAttached();
  await expect(page.locator("#status")).toHaveCount(0);
  expect(graphRequests).toEqual([]);
});

/** Selects an example's actual slide without depending on authoring order or text slides. */
async function selectSlide(page, name) {
  const iframe = page.locator(`iframe[data-example="tutorial/${name}"]`);
  const number = await iframe.evaluate((element) => {
    const section = element.closest("section");
    return [...document.querySelectorAll("section")].indexOf(section) + 1;
  });
  await page.evaluate((value) => { location.hash = String(value); }, number);
  const frame = iframe.contentFrame();
  await expect(frame.getByRole("textbox").first()).toBeVisible();
  return frame;
}

/** Finds the editable number field belonging to a named slider. */
function sliderInput(frame, name) {
  return frame.getByRole("textbox", {name, exact:true});
}

/** Commits a native keyboard edit through the actual shared bound control. */
async function writeSlider(frame, name, value) {
  const input = sliderInput(frame, name);
  await input.fill(String(value));
  await input.press("Tab");
}

/** Returns the nearest directed edge endpoint to a visible cell label. */
async function graphDirection(frame, name) {
  return frame.locator("#graph svg").evaluate((svg, cell) => {
    const label = [...svg.querySelectorAll("text.node-label")].find((element) => element.textContent === cell);
    const point = new DOMPoint(Number(label.getAttribute("x")), Number(label.getAttribute("y")))
      .matrixTransform(label.getScreenCTM());
    let nearest = { distance: Infinity, direction: "" };
    for (const line of svg.querySelectorAll("line[marker-end]")) {
      for (const [index, direction] of [[1, "source"], [2, "target"]]) {
        const endpoint = new DOMPoint(Number(line.getAttribute(`x${index}`)), Number(line.getAttribute(`y${index}`)))
          .matrixTransform(line.getScreenCTM());
        const distance = Math.hypot(endpoint.x - point.x, endpoint.y - point.y);
        if (distance < nearest.distance) nearest = { distance, direction };
      }
    }
    return nearest.direction;
  }, name);
}

/** Catches reactive feedback during real pointer interaction with a bound slider. */
test("dragging a bound slider updates the sheet without freezing", async ({ page }) => {
  await page.goto(host("basic_relationship", true));
  const slider = page.getByRole("slider", { name: "a", exact: true });
  await expect(slider).toHaveAttribute("aria-valuenow", "4");
  const box = await slider.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2, { steps: 4 });
  await page.mouse.up();
  const a = page.getByRole("textbox", { name: "a", exact: true });
  const b = page.getByRole("textbox", { name: "b", exact: true });
  await expect(a).not.toHaveValue("4");
  await expect.poll(async () => Number(await a.inputValue()) === 2 * Number(await b.inputValue())).toBe(true);
  const dragged = await a.inputValue();
  await expect(slider).toHaveAttribute("aria-valuenow", dragged);
  await expect(page.locator("#graph svg").getByText(dragged, { exact: true })).toBeVisible();
  await expect.poll(() => graphDirection(page, "a")).toBe("source");

  await slider.press("ArrowRight");
  await expect(a).toHaveValue(String(Number(dragged) + 1));
  await expect.poll(async () => Number(await a.inputValue()) === 2 * Number(await b.inputValue())).toBe(true);

  const track = slider.locator("xpath=..");
  const trackBox = await track.boundingBox();
  await page.mouse.click(trackBox.x + trackBox.width * 0.75, trackBox.y + trackBox.height / 2);
  await expect(a).not.toHaveValue(String(Number(dragged) + 1));
  await expect(slider).toHaveAttribute("aria-valuenow", await a.inputValue());
  await expect.poll(async () => Number(await a.inputValue()) === 2 * Number(await b.inputValue())).toBe(true);
});

/** Catches detached nudge controls and stepping that depends on displayed precision. */
test("number field integrates nudges and steps to zero-anchored values", async ({ page }) => {
  await page.goto(host("basic_relationship", true));
  const input = page.getByRole("textbox", { name: "a", exact: true });
  const up = page.getByRole("button", { name: "Increase a", exact: true });
  const down = page.getByRole("button", { name: "Decrease a", exact: true });
  await expect(up).toBeVisible();
  await expect(down).toBeVisible();
  const field = await input.boundingBox();
  const upBox = await up.boundingBox();
  const downBox = await down.boundingBox();
  expect(Math.abs(field.x + field.width - upBox.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(upBox.x - downBox.x)).toBeLessThanOrEqual(1);
  expect(Math.abs(upBox.y + upBox.height - downBox.y)).toBeLessThanOrEqual(1);

  await input.fill("3.45");
  await input.press("Tab");
  await up.click();
  await expect(input).toHaveValue("4");
  await expect(page.getByRole("textbox", { name: "b", exact: true })).toHaveValue("2");
  await input.fill("3.45");
  await input.press("ArrowDown");
  await expect(input).toHaveValue("3");
  await down.click();
  await expect(input).toHaveValue("2");
});

/** Verifies that an unfocused numeric frame paints above the input background. */
test("number field keeps its frame when unfocused", async ({ page }) => {
  await page.goto(host("basic_relationship", true));
  const input = page.getByRole("textbox", { name: "a", exact: true });
  await expect(input).toHaveValue("4");
  await page.mouse.move(0, 0);
  const controls = input.locator("xpath=..");
  const png = await controls.screenshot({ scale: "css" });
  const pixels = await page.evaluate(async (data) => {
    const image = new Image();
    image.src = `data:image/png;base64,${data}`;
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext("2d");
    context.drawImage(image, 0, 0);
    const middle = Math.floor(image.height / 2);
    return [0, 4].map((x) => [...context.getImageData(x, middle, 1, 1).data]);
  }, png.toString("base64"));
  expect(pixels[0]).not.toEqual(pixels[1]);
});

/** Catches nudge controls that permit stepping past either live bound. */
test("number field disables out-of-range nudges", async ({ page }) => {
  await page.goto(host("basic_relationship", true));
  const input = page.getByRole("textbox", { name: "a", exact: true });
  const up = page.getByRole("button", { name: "Increase a", exact: true });
  const down = page.getByRole("button", { name: "Decrease a", exact: true });
  await input.fill("0");
  await input.press("Tab");
  await expect(down).toBeDisabled();
  await expect(up).toBeEnabled();
  await expect(down).toHaveCSS("opacity", "0.5");
  await expect(up).toHaveCSS("opacity", "1");
  await input.press("ArrowDown");
  await expect(input).toHaveValue("0");
  await input.fill("200");
  await input.press("Tab");
  await expect(up).toBeDisabled();
  await expect(down).toBeEnabled();
  await input.press("ArrowUp");
  await expect(input).toHaveValue("200");
  await down.click();
  await expect(input).toHaveValue("199");
  await expect(up).toBeEnabled();
});

/** Catches a ranged cell stacking its slider above or beside the wrong field edge. */
test("ranged slider sits left of its number field", async ({ page }) => {
  await page.goto(host("basic_relationship", true));
  const thumb = page.getByRole("slider", { name: "a", exact: true });
  await expect(thumb).toBeVisible();
  const track = thumb.locator('xpath=ancestor-or-self::*[@data-orientation][1]');
  const input = page.getByRole("textbox", { name: "a", exact: true });
  const inputId = await input.getAttribute("id");
  const label = page.locator(`label[for="${inputId}"]`);
  const trackBox = await track.boundingBox();
  const inputBox = await input.boundingBox();
  const labelBox = await label.boundingBox();
  expect(trackBox.x + trackBox.width).toBeLessThanOrEqual(inputBox.x);
  const trackCenter = trackBox.y + trackBox.height / 2;
  const inputCenter = inputBox.y + inputBox.height / 2;
  expect(Math.abs(trackCenter - inputCenter)).toBeLessThan(4);
  expect(labelBox.y + labelBox.height).toBeLessThanOrEqual(Math.min(trackBox.y, inputBox.y));
});

/** Catches control sizing, help leakage, and inspector overflow drifting from dioxus-components. */
test("controls match dioxus-components sizes", async ({ page }) => {
  await page.goto(host("basic_relationship", true));
  const input = page.getByRole("textbox", { name: "a", exact: true });
  await expect(input).toBeVisible();
  const inputId = await input.getAttribute("id");
  const label = page.locator(`label[for="${inputId}"]`);
  // The publishing host scales every embedded `rem` length by `--adam-rem`, so assert the
  // upstream dioxus-components sizes (label 0.8rem, label-to-field gap 0.5rem) relative to the
  // host-resolved rem rather than a fixed 16px root. Absolute, px-based sizes (input height)
  // stay fixed regardless of the scale.
  const rem = await label.evaluate((element) => {
    const probe = document.createElement("div");
    probe.style.cssText = "position:absolute;width:var(--adam-rem,1rem)";
    element.appendChild(probe);
    const width = probe.getBoundingClientRect().width;
    probe.remove();
    return width;
  });
  const fontSize = await label.evaluate((element) => parseFloat(getComputedStyle(element).fontSize));
  expect(Math.abs(fontSize - 0.8 * rem)).toBeLessThan(0.5);
  // The native field keeps its upstream, px-based text size and padding regardless of the host
  // rem scale (dioxus-components native input: ~13.33px text, 8px/12px padding).
  const field = await input.evaluate((element) => {
    const cs = getComputedStyle(element);
    return { font: parseFloat(cs.fontSize), pt: cs.paddingTop, pl: cs.paddingLeft };
  });
  expect(Math.abs(field.font - 13.333)).toBeLessThan(0.5);
  expect(field.pt).toBe("8px");
  expect(field.pl).toBe("12px");
  // The framed number input matches the upstream dioxus-components 31px height at a 16px
  // effective rem; scale the expectation by the host-resolved rem so the check holds under
  // any `--adam-rem`.
  const inputHeight = await input.evaluate((element) => element.getBoundingClientRect().height);
  expect(Math.abs(inputHeight - 31 * (rem / 16))).toBeLessThanOrEqual(1);
  const inputBox = await input.boundingBox();
  const labelBox = await label.boundingBox();
  const gap = inputBox.y - (labelBox.y + labelBox.height);
  expect(Math.abs(gap - 0.5 * rem)).toBeLessThanOrEqual(1);
  const help = page.locator(`[id="${inputId}-help"]`);
  const helpBox = await help.boundingBox();
  expect(helpBox.width).toBeLessThanOrEqual(1);
  expect(helpBox.height).toBeLessThanOrEqual(1);
  const overflow = await page.locator(".adam-sheet-inspector").evaluate(
    (element) => element.scrollWidth - element.clientWidth,
  );
  expect(overflow).toBeLessThanOrEqual(0);
});

/** Catches broken write/propagate bridging and uneditable computed outputs. */
test("live output follows an input edit", async ({ page }) => {
  await page.goto("./");
  const frame = await selectSlide(page, "basic_output");
  await writeSlider(frame, "width", 30);
  await expect(sliderInput(frame, "area")).toHaveValue("600");
  await expect(frame.locator('#graph svg').getByText("600", { exact: true })).toBeVisible();
});

/** Catches missing bidirectional propagation or graph updates. */
test("editing either side reverses a relationship and updates its graph", async ({ page }) => {
  await page.goto("./");
  const frame = await selectSlide(page, "basic_relationship");
  await writeSlider(frame, "a", 40);
  await expect(sliderInput(frame, "b")).toHaveValue("20");
  await expect(frame.locator("#graph svg").getByText("40", { exact: true })).toBeVisible();
  await expect.poll(() => graphDirection(frame, "a")).toBe("source");
  await writeSlider(frame, "b", 15);
  await expect(sliderInput(frame, "a")).toHaveValue("30");
  await expect(frame.locator("#graph svg").getByText("30", { exact: true })).toBeVisible();
  await expect.poll(() => graphDirection(frame, "a")).toBe("target");
});

/** Catches resetting derived source memory when navigating the chained example. */
test("inequality restores prior values after releasing a constraint", async ({ page }) => {
  await page.goto("./");
  const frame = await selectSlide(page, "inequality");
  await writeSlider(frame, "a", 100);
  await expect(sliderInput(frame, "b")).toHaveValue("100");
  await expect(sliderInput(frame, "c")).toHaveValue("100");
  await writeSlider(frame, "a", 0);
  await expect(sliderInput(frame, "b")).toHaveValue("20");
  await expect(sliderInput(frame, "c")).toHaveValue("30");
});

/** Catches checkbox events failing to activate or deactivate relationships. */
test("conditional toggle links and releases cells", async ({ page }) => {
  await page.goto("./");
  const frame = await selectSlide(page, "constrain");
  await expect(frame.locator("#graph svg line.link[marker-end]")).toHaveCount(1);
  await frame.getByRole("checkbox", { name: "constrain" }).check();
  await expect(frame.locator("#graph svg line.link[marker-end]")).toHaveCount(3);
  await expect(sliderInput(frame, "a")).toHaveValue("10");
  await writeSlider(frame, "a", 20);
  await expect(sliderInput(frame, "b")).toHaveValue("20");
  await frame.getByRole("checkbox", { name: "constrain" }).uncheck();
  await expect(frame.locator("#graph svg line.link[marker-end]")).toHaveCount(1);
  await writeSlider(frame, "a", 30);
  await expect(sliderInput(frame, "b")).toHaveValue("20");
});

/** Catches forced outputs staying editable or losing their preserved source value. */
test("forced conditional value disables editing and restores its source", async ({ page }) => {
  await page.goto("./");
  const frame = await selectSlide(page, "conditional_forced");
  const input = sliderInput(frame, "a");
  await frame.getByRole("checkbox", { name: "constrain" }).check();
  await expect(input).toHaveValue("42");
  await expect(input).not.toBeEditable();
  await frame.getByRole("checkbox", { name: "constrain" }).uncheck();
  await expect(input).toHaveValue("5");
  await expect(input).toBeEditable();
});

/** Catches omitted filter or requirement diagnostics in the embedded host. */
test("filtering and requirement diagnostics remain visible", async ({ page }) => {
  await page.goto("./");
  const filter = await selectSlide(page, "clamp_demo");
  await writeSlider(filter, "level", 150);
  await expect(sliderInput(filter, "level")).toHaveValue("100");
  const diagnostic = await selectSlide(page, "requirements_filter_diagnostic");
  await expect(diagnostic.getByRole("alert").filter({hasText:/filter violation/})).toBeVisible();
  await writeSlider(diagnostic, "height", 50);
  await expect(diagnostic.getByRole("alert").filter({hasText:/filter violation/})).toHaveCount(0);
  const requirements = await selectSlide(page, "area_with_requirement");
  await writeSlider(requirements, "width", 100);
  await writeSlider(requirements, "height", 100);
  await expect(requirements.getByText(/not_too_big/)).toBeVisible();
});

/** Catches input/graph events advancing Marp and slide switches recreating sheets. */
test("frame interactions stay isolated and navigation preserves edits", async ({ page }) => {
  await page.goto("./");
  const first = await selectSlide(page, "first_sheet");
  const initialURL = page.url();
  const width = first.getByRole("textbox", { name: "width", exact: true });
  await width.fill("123");
  await width.press("ArrowRight");
  await width.press("Tab");
  await expect(width).toHaveValue("123");
  await expect(page).toHaveURL(initialURL);
  await page.getByRole("heading", { name: "A first sheet" }).click();
  await page.keyboard.press("ArrowRight");
  await expect(page).not.toHaveURL(initialURL);
  await selectSlide(page, "first_sheet");
  await expect(width).toHaveValue("123");
  const bounds = await first.locator("#graph svg").boundingBox();
  expect(bounds.width).toBeGreaterThan(0);
  expect(bounds.height).toBeGreaterThan(0);
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 + 20, bounds.y + bounds.height / 2 + 20);
  await page.mouse.up();
  await expect(page).toHaveURL(initialURL);
  await page.setViewportSize({ width: 960, height: 540 });
  await selectSlide(page, "basic_output");
  await selectSlide(page, "first_sheet");
  await expect(width).toHaveValue("123");
  await expect(first.locator("#graph svg")).toBeVisible();
});

/** Catches missing local sources and treating a text-only introduction as an example. */
test("text and deck-local example slides render independently", async ({ page }) => {
  await page.goto("./");
  await expect(page.getByRole("heading", { name: "Text-only introduction" })).toBeVisible();
  const introduction = page.getByRole("heading", { name: "Text-only introduction" })
    .locator("xpath=ancestor::section");
  await expect(introduction.locator("iframe")).toHaveCount(0);
  await page.goto("example.html?example=local%2Facceptance&graph=0");
  await expect(page.getByRole("textbox", { name: "width", exact: true })).toBeVisible();
  await expect(page.locator("#status")).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
});

/** Catches failures in the actual authored deck independently of the acceptance fixture. */
test("the authored deck mounts every selected example", async ({ page }) => {
  await page.goto("authored.html");
  const iframes = page.locator("iframe[data-example]");
  await expect(iframes).toHaveCount(10);
  expect(await iframes.evaluateAll((frames) => frames.map((frame) => frame.dataset.example))).toEqual([
    "tutorial/first_sheet",
    "tutorial/clamp_demo",
    "tutorial/basic_output",
    "tutorial/basic_relationship",
    "tutorial/inequality",
    "tutorial/constrain",
    "tutorial/conditional_forced",
    "tutorial/requirements_filter_diagnostic",
    "tutorial/area_with_requirement",
    "local/three_way",
  ]);
  for (let index = 0; index < await iframes.count(); index++) {
    const frame = iframes.nth(index).contentFrame();
    await expect(frame.locator("#status")).toHaveCount(0);
    await expect(frame.locator(".adam-theme").first()).toBeAttached();
    if (await iframes.nth(index).getAttribute("data-example") === "tutorial/requirements_filter_diagnostic") {
      await expect(frame.getByRole("alert")).toHaveCount(1);
      await expect(frame.getByRole("alert")).toContainText("filter violation");
    } else {
      await expect(frame.getByRole("alert")).toHaveCount(0);
    }
  }
});

/** Catches the inspector losing its headerless, centered card presentation. */
test("inspector is a centered card", async ({ page }) => {
  await page.goto(host("basic_relationship", true));
  const card = page.locator('[data-slot="card"]');
  await expect(card).toHaveCount(1);
  expect(await page.locator("#inspector h2").count()).toBe(0);
  const cardBox = await card.boundingBox();
  expect(cardBox.width).toBeLessThanOrEqual(384);
  const contentCenter = await page.locator("#inspector").evaluate((element) => {
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    const left = rect.left + parseFloat(style.paddingLeft);
    const right = rect.right - parseFloat(style.paddingRight);
    return (left + right) / 2;
  });
  expect(Math.abs(cardBox.x + cardBox.width / 2 - contentCenter)).toBeLessThanOrEqual(2);
  const rows = page.locator("#inspector .adam-cell-row");
  const count = await rows.count();
  expect(count).toBeGreaterThan(1);
  const boxes = [];
  for (let index = 0; index < count; index++) {
    boxes.push(await rows.nth(index).boundingBox());
  }
  for (let index = 1; index < boxes.length; index++) {
    const delta = boxes[index].y - boxes[index - 1].y;
    expect(Math.abs(delta - (boxes[index - 1].height + 24))).toBeLessThanOrEqual(1);
  }
});
