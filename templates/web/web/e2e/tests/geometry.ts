import { expect, type Locator, type Page } from '@playwright/test';

export const BREAKPOINT_AUDIT_VIEWPORTS = [
  { name: 'compact-short', width: 320, height: 568 },
  { name: 'compact-normal', width: 320, height: 720 },
  { name: 'below-wide-short', width: 1023, height: 568 },
  { name: 'below-wide-normal', width: 1023, height: 720 },
  { name: 'wide-short', width: 1024, height: 568 },
  { name: 'wide-normal', width: 1024, height: 720 },
] as const;

interface LayoutRectangle {
  readonly height: number;
  readonly width: number;
  readonly x: number;
  readonly y: number;
}

export async function setAuditViewport(
  page: Page,
  viewport: (typeof BREAKPOINT_AUDIT_VIEWPORTS)[number],
): Promise<void> {
  await page.setViewportSize(viewport);
  await expect
    .poll(() =>
      page.evaluate(
        ({ height, width }) => window.innerHeight === height && window.innerWidth === width,
        viewport,
      ),
    )
    .toBe(true);
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() =>
          requestAnimationFrame(() => {
            resolve();
          }),
        ),
      ),
  );
}

export async function expectNoHorizontalDocumentOverflow(page: Page): Promise<void> {
  const layout = await page.evaluate(() => {
    const viewportWidth = document.documentElement.clientWidth;
    const overflow = document.documentElement.scrollWidth - viewportWidth;
    const largestElements = Array.from(document.querySelectorAll<HTMLElement>('body *'))
      .map((element) => {
        const rectangle = element.getBoundingClientRect();
        const leftOverflow = Math.max(0, -rectangle.left);
        const rightOverflow = Math.max(0, rectangle.right - viewportWidth);
        const id = element.id.length === 0 ? '' : `#${element.id}`;
        const classes = [...element.classList].map((name) => `.${name}`).join('');
        return {
          element: `${element.tagName.toLowerCase()}${id}${classes}`,
          overflow: Math.max(leftOverflow, rightOverflow),
          width: rectangle.width,
        };
      })
      .filter(({ width }) => width > 0)
      .sort((left, right) => right.overflow - left.overflow || right.width - left.width)
      .slice(0, 10);

    return { largestElements, overflow, viewportWidth };
  });

  expect(
    layout.overflow,
    `viewport width ${String(layout.viewportWidth)}; largest elements ${JSON.stringify(layout.largestElements)}`,
  ).toBeLessThanOrEqual(0);
}

export async function expectRectanglesDoNotIntersect(
  target: Locator,
  fixedNavigation: Locator,
): Promise<void> {
  await target.scrollIntoViewIfNeeded();
  const targetRectangle = await visibleRectangle(target);
  const navigationRectangle = await visibleRectangle(fixedNavigation);
  expect(
    rectanglesIntersect(targetRectangle, navigationRectangle),
    `target ${JSON.stringify(targetRectangle)} intersects fixed navigation ${JSON.stringify(navigationRectangle)}`,
  ).toBe(false);
}

export async function expectInsideScrollContainer(
  target: Locator,
  scrollContainer: Locator,
): Promise<void> {
  await target.scrollIntoViewIfNeeded();
  const targetRectangle = await visibleRectangle(target);
  const containerRectangle = await visibleRectangle(scrollContainer);
  const tolerance = 1;

  expect(targetRectangle.x).toBeGreaterThanOrEqual(containerRectangle.x - tolerance);
  expect(targetRectangle.y).toBeGreaterThanOrEqual(containerRectangle.y - tolerance);
  expect(targetRectangle.x + targetRectangle.width).toBeLessThanOrEqual(
    containerRectangle.x + containerRectangle.width + tolerance,
  );
  expect(targetRectangle.y + targetRectangle.height).toBeLessThanOrEqual(
    containerRectangle.y + containerRectangle.height + tolerance,
  );
}

export async function expectMinimumInteractiveTargetSize(
  targets: Locator,
  minimumCssPixels: number,
): Promise<void> {
  const count = await targets.count();
  expect(count).toBeGreaterThan(0);
  let visibleCount = 0;

  for (let index = 0; index < count; index += 1) {
    const target = targets.nth(index);
    if (!(await target.isVisible())) continue;
    visibleCount += 1;
    const rectangle = await visibleRectangle(target);
    expect(
      rectangle.width,
      `interactive target ${String(index)} is too narrow`,
    ).toBeGreaterThanOrEqual(minimumCssPixels);
    expect(
      rectangle.height,
      `interactive target ${String(index)} is too short`,
    ).toBeGreaterThanOrEqual(minimumCssPixels);
  }

  expect(visibleCount).toBeGreaterThan(0);
}

async function visibleRectangle(locator: Locator): Promise<LayoutRectangle> {
  await expect(locator).toBeVisible();
  const rectangle = await locator.boundingBox();
  if (rectangle === null) throw new Error('visible element has no layout rectangle');
  return rectangle;
}

function rectanglesIntersect(first: LayoutRectangle, second: LayoutRectangle): boolean {
  return (
    first.x < second.x + second.width &&
    first.x + first.width > second.x &&
    first.y < second.y + second.height &&
    first.y + first.height > second.y
  );
}
