import { expect, test, type Page, type Request } from "@playwright/test";

/** Inertia XHR/fetch requests made by the client while `fn` runs. */
async function inertiaRequests(page: Page, fn: () => Promise<unknown>): Promise<Request[]> {
  const seen: Request[] = [];
  const onRequest = (r: Request) => {
    if (r.headers()["x-inertia"]) seen.push(r);
  };
  page.on("request", onRequest);
  try {
    await fn();
  } finally {
    page.off("request", onRequest);
  }
  return seen;
}

/** Marks the current document; a full page load removes the mark. */
const markDocument = (page: Page) => page.evaluate(() => ((window as any).__e2e = true));
const sameDocument = (page: Page) => page.evaluate(() => (window as any).__e2e === true);

test("first visit is server-rendered and hydrates without errors", async ({ page }) => {
  const errors: string[] = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("response", (r) => r.status() >= 400 && errors.push(`${r.status()} ${r.url()}`));

  const res = await page.request.get("/about");
  const html = await res.text();
  expect(html).toContain('data-server-rendered="true"');
  expect(html).toContain("<title data-inertia=\"\">About · Loco + Inertia</title>");

  await page.goto("/about");
  await expect(page).toHaveTitle("About · Loco + Inertia");
  await expect(page.getByTestId("app-name")).toHaveText("Loco + Inertia");
  // A hydration mismatch shows up as a console error.
  await page.waitForTimeout(300);
  expect(errors).toEqual([]);
});

test("links navigate with JSON responses, not page loads", async ({ page }) => {
  await page.goto("/");
  await markDocument(page);
  const reqs = await inertiaRequests(page, async () => {
    await page.getByRole("link", { name: "About" }).click();
    await expect(page.locator("h1")).toBeVisible();
    await expect(page).toHaveURL(/\/about$/);
  });
  expect(reqs).toHaveLength(1);
  const res = await reqs[0].response();
  expect(res?.headers()["x-inertia"]).toBe("true");
  expect(res?.headers()["content-type"]).toContain("application/json");
  expect((await res?.json()).component).toBe("About");
  expect(await sameDocument(page)).toBe(true);
  await expect(page).toHaveTitle("About · Loco + Inertia");
});

test("a stale asset version makes the client reload the page", async ({ page }) => {
  await page.goto("/");
  await markDocument(page);
  // Pretend the client was loaded from an older build.
  await page.route("**/about", (route) =>
    route.continue({ headers: { ...route.request().headers(), "x-inertia-version": "old-build" } }),
  );
  const conflict = page.waitForResponse((r) => r.url().endsWith("/about") && r.status() === 409);
  await page.getByRole("link", { name: "About" }).click();
  expect((await conflict).headers()["x-inertia-location"]).toBe("/about");
  await expect(page).toHaveURL(/\/about$/);
  await expect(page).toHaveTitle("About · Loco + Inertia");
  expect(await sameDocument(page)).toBe(false);
});

test("once props are sent once and skipped afterwards", async ({ page }) => {
  await page.goto("/");
  const loadedAt = await page.getByTestId("loaded-at").textContent();
  const reqs = await inertiaRequests(page, async () => {
    await page.getByRole("link", { name: "About" }).click();
    await expect(page).toHaveURL(/\/about$/);
    await page.getByRole("link", { name: "Home" }).click();
    await expect(page).toHaveURL(/\/$/);
  });
  for (const r of reqs) {
    expect(r.headers()["x-inertia-except-once-props"]).toContain("loaded_at");
    const body = await (await r.response())!.json();
    expect(body.props).not.toHaveProperty("loaded_at");
    expect(body.onceProps.loaded_at.prop).toBe("loaded_at");
  }
  await expect(page.getByTestId("loaded-at")).toHaveText(loadedAt!);
});

test("deferred props load after the first render", async ({ page }) => {
  const reqs = await inertiaRequests(page, async () => {
    await page.goto("/feed");
    await expect(page.getByText("Loading stats…")).toBeVisible();
    await expect(page.getByTestId("stats")).toHaveText("100 posts in total (computed in 600 ms)");
  });
  expect(reqs.map((r) => r.headers()["x-inertia-partial-data"])).toContain("stats");
});

test("optional props load only when asked for", async ({ page }) => {
  await page.goto("/feed");
  await expect(page.getByTestId("categories")).toHaveCount(0);
  await page.getByRole("button", { name: "Load categories" }).click();
  await expect(page.getByTestId("categories")).toHaveText("rust, loco, inertia");
});

test("infinite scroll appends pages without duplicates and resets on filter", async ({ page }) => {
  await page.goto("/feed");
  await expect(page.getByTestId("post")).toHaveCount(10);
  for (let loaded = 20; loaded <= 40; loaded += 10) {
    await page.getByTestId("post").last().scrollIntoViewIfNeeded();
    await expect(page.getByTestId("post")).toHaveCount(loaded);
  }
  const titles = await page.getByTestId("post").allTextContents();
  expect(new Set(titles).size).toBe(titles.length);
  expect(titles.slice(0, 3)).toEqual(["Post #1", "Post #2", "Post #3"]);
  // The client keeps the page in view in the URL.
  await expect(page).toHaveURL(/page=[2-4]/);

  // Controlled by the `even_only` prop: it flips once the server answers.
  await page.getByTestId("even").click();
  await expect(page.getByTestId("even")).toBeChecked();
  await expect(page.getByTestId("post").first()).toHaveText("Post #2");
  const even = await page.getByTestId("post").allTextContents();
  expect(even.length).toBeLessThanOrEqual(20);
  expect(even.every((t) => Number(t.replace("Post #", "")) % 2 === 0)).toBe(true);
});

test("precognition validates fields live without submitting", async ({ page }) => {
  await page.goto("/contact");
  const reqs = await inertiaRequests(page, async () => {
    await page.getByTestId("input-email").fill("not-an-email");
    await page.getByTestId("input-email").blur();
    await expect(page.getByTestId("error-email")).toHaveText("That does not look like an email address.");
  });
  await page.getByTestId("input-email").fill("ann@example.com");
  await page.getByTestId("input-email").blur();
  await expect(page.getByTestId("valid-email")).toBeVisible();
  // Precognition requests are not Inertia visits; the handler never ran, so no flash.
  expect(reqs).toHaveLength(0);
  await expect(page.getByTestId("flash")).toHaveCount(0);
});

test("invalid submits show errors; valid ones redirect with a one-time flash", async ({ page }) => {
  await page.goto("/contact");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByTestId("error-name")).toHaveText("Please tell us your name.");
  await expect(page.getByTestId("error-message")).toBeVisible();

  await page.getByTestId("input-name").fill("Ann");
  await page.getByTestId("input-email").fill("ann@example.com");
  await page.getByTestId("input-message").fill("Hello there, Loco!");
  await page.getByRole("button", { name: "Send" }).click();
  await expect(page.getByTestId("flash")).toHaveText("Thanks Ann, we got your message!");
  await expect(page.getByTestId("error-name")).toHaveCount(0);

  await page.getByRole("link", { name: "About" }).click();
  await expect(page).toHaveURL(/\/about$/);
  await expect(page.getByTestId("flash")).toHaveCount(0);
  await page.reload();
  await expect(page.getByTestId("flash")).toHaveCount(0);
});

test("the contact page is encrypted in the browser history", async ({ page }) => {
  await page.goto("/contact");
  const state = await page.evaluate(() => window.history.state);
  expect(state?.page).toBeDefined();
  // Encrypted pages are stored as bytes, not as the page object.
  expect(state.page.component).toBeUndefined();

  await page.getByRole("link", { name: "About" }).click();
  await expect(page).toHaveURL(/\/about$/);
  expect((await page.evaluate(() => window.history.state)).page.component).toBe("About");
});

test("unsafe requests without the CSRF header are rejected", async ({ page }) => {
  await page.goto("/contact");
  const res = await page.request.post("/contact", {
    data: { name: "Ann", email: "ann@example.com", message: "Hello there, Loco!" },
    headers: { "X-Inertia": "true" },
  });
  expect(res.status()).toBe(419);
});

test("multipart uploads with files", async ({ page }) => {
  await page.goto("/upload");
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.getByTestId("name").fill("Ann");
  await page.getByTestId("avatar").setInputFiles({ name: "me.png", mimeType: "image/png", buffer: png });
  await page.getByTestId("photos").setInputFiles([
    { name: "a.png", mimeType: "image/png", buffer: png },
    { name: "b.png", mimeType: "image/png", buffer: png },
  ]);
  await page.getByRole("button", { name: "Upload" }).click();
  await expect(page.getByTestId("flash")).toHaveText(
    `Thanks Ann: me.png (image/png, ${png.length} bytes); 2 photo(s): a.png (image/png, ${png.length} bytes), b.png (image/png, ${png.length} bytes)`,
  );

  await page.getByTestId("name").fill("Ann");
  await page.getByTestId("avatar").setInputFiles({ name: "x.txt", mimeType: "text/plain", buffer: Buffer.from("hi") });
  await page.getByRole("button", { name: "Upload" }).click();
  await expect(page.getByTestId("error-avatar")).toHaveText("Only images are allowed.");
});
