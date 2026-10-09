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

/** Catches missing live mounts, duplicate slides, source drift, and runtime network dependencies. */
test("nine canonical examples mount without external runtime requests", async ({ page }, testInfo) => {
  const unexpected = [];
  page.on("pageerror", (error) => unexpected.push(error.message));
  page.on("request", (request) => {
    if (/^https?:/.test(request.url()) && new URL(request.url()).hostname !== "127.0.0.1") {
      unexpected.push(request.url());
    }
  });
  await page.goto("./");
  await expect(page.locator("iframe[data-example]")).toHaveCount(9);
  for (const [index, name] of names.entries()) {
    await page.evaluate((number) => { location.hash = String(number); }, index + 1);
    const iframe = page.locator(`iframe[data-example="tutorial/${name}"]`);
    const frame = iframe.contentFrame();
    await expect(frame.locator("sp-theme")).toBeAttached();
    await expect(frame.locator("#status")).toHaveCount(0);
    await expect(frame.getByRole("textbox").first()).toBeVisible();
    expect(await frame.locator("sp-slider, sp-number-field, sp-checkbox").count()).toBeGreaterThan(0);
    expect(await frame.locator("sp-slider, sp-number-field, sp-checkbox").first()
      .evaluate((element) => Boolean(element.shadowRoot))).toBe(true);
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
      const frame = await selectSlide(page, index + 1);
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

/** Catches slide numbers overlapping live panes instead of occupying the bottom margin. */
test("slide numbers stay below the example panes", async ({ page }, testInfo) => {
  await page.goto("./");
  for (const width of [1280, 960]) {
    await page.setViewportSize({ width, height: width * 9 / 16 });
    for (const [index, name] of names.entries()) {
      await selectSlide(page, index + 1);
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
  ["Spectrum", "swc.js"],
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
  ["Spectrum", "swc.js", false, 'throw new Error("Spectrum execution failed");'],
  ["D3", "d3.v7.min.js", true, 'throw new Error("D3 execution failed");'],
  ["graph driver", "graph.js", true, 'throw new Error("Graph execution failed");'],
  ["Spectrum registration", "swc.js", false, ""],
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
        if (!window.d3 || !window.beginGraph || !customElements.get("sp-slider")) throw new Error("Dependencies not ready");
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
    if (/\/(graph\.js|d3\.v7\.min\.js)$/.test(request.url())) graphRequests.push(request.url());
  });
  await page.goto(host("clamp_demo"));
  await expect(page.locator("sp-slider")).toBeAttached();
  await expect(page.locator("#status")).toHaveCount(0);
  expect(graphRequests).toEqual([]);
});

/** Selects a presentation slide without reloading its mounted frames. */
async function selectSlide(page, number) {
  await page.evaluate((value) => { location.hash = String(value); }, number);
  const frame = page.locator(`iframe[data-example="tutorial/${names[number - 1]}"]`).contentFrame();
  await expect(frame.getByRole("textbox").first()).toBeVisible();
  return frame;
}

/** Finds the editable number field belonging to a named slider. */
function sliderInput(frame, name) {
  return frame.locator(`sp-slider[label="${name}"]`).getByRole("textbox");
}

/** Commits a normal keyboard edit through the real Spectrum control. */
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

/** Catches broken write/propagate bridging and uneditable computed outputs. */
test("live output follows an input edit", async ({ page }) => {
  await page.goto("./");
  const frame = await selectSlide(page, 3);
  await writeSlider(frame, "width", 30);
  await expect(frame.getByText("600", { exact: true })).toBeVisible();
  await expect(frame.locator('#graph svg').getByText("600", { exact: true })).toBeVisible();
});

/** Catches missing bidirectional propagation or graph updates. */
test("editing either side reverses a relationship and updates its graph", async ({ page }) => {
  await page.goto("./");
  const frame = await selectSlide(page, 4);
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
  const frame = await selectSlide(page, 5);
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
  const frame = await selectSlide(page, 6);
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
  const frame = await selectSlide(page, 7);
  const input = sliderInput(frame, "a");
  await frame.getByRole("checkbox", { name: "constrain" }).check();
  await expect(input).toHaveValue("42");
  await expect(input).toBeDisabled();
  await frame.getByRole("checkbox", { name: "constrain" }).uncheck();
  await expect(input).toHaveValue("5");
  await expect(input).toBeEnabled();
});

/** Catches omitted filter or requirement diagnostics in the embedded host. */
test("filtering and requirement diagnostics remain visible", async ({ page }) => {
  await page.goto("./");
  const filter = await selectSlide(page, 2);
  await writeSlider(filter, "level", 150);
  await expect(sliderInput(filter, "level")).toHaveValue("100");
  const diagnostic = await selectSlide(page, 8);
  await expect(diagnostic.getByText(/filter violation.*width/)).toBeVisible();
  await writeSlider(diagnostic, "height", 50);
  await expect(diagnostic.getByText(/filter violation.*width/)).toHaveCount(0);
  const requirements = await selectSlide(page, 9);
  await writeSlider(requirements, "width", 100);
  await writeSlider(requirements, "height", 100);
  await expect(requirements.getByText(/not_too_big/)).toBeVisible();
});

/** Catches input/graph events advancing Marp and slide switches recreating sheets. */
test("frame interactions stay isolated and navigation preserves edits", async ({ page }) => {
  await page.goto("./#1");
  const first = await selectSlide(page, 1);
  const width = first.getByRole("textbox", { name: "width", exact: true });
  await width.fill("123");
  await width.press("ArrowRight");
  await width.press("Tab");
  await expect(width).toHaveValue("123");
  await expect(page).toHaveURL(/#1$/);
  await page.getByRole("heading", { name: "A first sheet" }).click();
  await page.keyboard.press("ArrowRight");
  await expect(page).toHaveURL(/#2$/);
  await selectSlide(page, 1);
  await expect(width).toHaveValue("123");
  const bounds = await first.locator("#graph svg").boundingBox();
  expect(bounds.width).toBeGreaterThan(0);
  expect(bounds.height).toBeGreaterThan(0);
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width / 2 + 20, bounds.y + bounds.height / 2 + 20);
  await page.mouse.up();
  await expect(page).toHaveURL(/#1$/);
  await page.setViewportSize({ width: 960, height: 540 });
  await selectSlide(page, 3);
  await selectSlide(page, 1);
  await expect(width).toHaveValue("123");
  await expect(first.locator("#graph svg")).toBeVisible();
});
