import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  createApp,
  defineComponent,
  h,
  nextTick,
  shallowRef,
  type App,
  type ComponentPublicInstance,
} from 'vue'

import type { TieredPricingConfig } from '@/api/endpoints/types'
import TieredPricingEditor from '../TieredPricingEditor.vue'

interface TieredPricingEditorExposed extends ComponentPublicInstance {
  getFinalPricing: () => TieredPricingConfig
  getValidationError: () => string | null
}

const mountedApps: Array<{ app: App, root: HTMLElement }> = []

function mountEditor(modelValue: TieredPricingConfig) {
  const root = document.createElement('div')
  document.body.appendChild(root)
  const onUpdate = vi.fn()
  const currentModelValue = shallowRef(modelValue)
  let editor: TieredPricingEditorExposed | null = null

  const app = createApp(defineComponent({
    setup() {
      return () => h(TieredPricingEditor, {
        ref: (instance: unknown) => {
          editor = instance as TieredPricingEditorExposed | null
        },
        modelValue: currentModelValue.value,
        'onUpdate:modelValue': onUpdate,
      })
    },
  }))

  app.mount(root)
  mountedApps.push({ app, root })

  return {
    root,
    onUpdate,
    getFinalPricing: () => {
      if (!editor) throw new Error('TieredPricingEditor ref was not mounted')
      return editor.getFinalPricing()
    },
    getValidationError: () => {
      if (!editor) throw new Error('TieredPricingEditor ref was not mounted')
      return editor.getValidationError()
    },
  }
}

function button(root: HTMLElement, testId: string): HTMLButtonElement {
  const element = root.querySelector(`[data-testid="${testId}"]`)
  if (!(element instanceof HTMLButtonElement)) throw new Error(`missing button ${testId}`)
  return element
}

function input(root: HTMLElement, selector: string): HTMLInputElement {
  const element = root.querySelector(selector)
  if (!(element instanceof HTMLInputElement)) throw new Error(`missing input ${selector}`)
  return element
}

function typeInto(element: HTMLInputElement, value: string) {
  element.value = value
  element.dispatchEvent(new Event('input'))
}

afterEach(() => {
  for (const { app, root } of mountedApps.splice(0)) {
    app.unmount()
    root.remove()
  }
})

const BASE_TIERS = [{ up_to: null, input_price_per_1m: 5, output_price_per_1m: 30 }]
const SHANGHAI_PEAK = {
  timezone: 'Asia/Shanghai',
  windows: [{
    id: 'workday-morning-peak',
    weekdays: ['monday', 'tuesday', 'wednesday', 'thursday', 'friday'],
    start: '09:00',
    end: '12:00',
    price_multiplier: 2,
  }],
}

describe('TieredPricingEditor time based pricing', () => {
  it('round-trips a configured peak window byte for byte', () => {
    const { getFinalPricing } = mountEditor({
      tiers: BASE_TIERS,
      time_pricing: SHANGHAI_PEAK,
    } as TieredPricingConfig)

    expect(getFinalPricing().time_pricing).toEqual(SHANGHAI_PEAK)
  })

  it('leaves a catalog without time pricing untouched', () => {
    const { getFinalPricing } = mountEditor({ tiers: BASE_TIERS } as TieredPricingConfig)

    expect('time_pricing' in getFinalPricing()).toBe(false)
  })

  it('enabling the switch alone yields a savable window that reaches the form', async () => {
    const { root, onUpdate, getFinalPricing } = mountEditor({ tiers: BASE_TIERS } as TieredPricingConfig)

    button(root, 'time-pricing-enabled').click()
    await nextTick()

    // Turning the switch on must not need a second edit inside the window: until the parent form
    // learns about the window, saving silently writes a catalog without `time_pricing`.
    expect(onUpdate).toHaveBeenLastCalledWith(expect.objectContaining({
      time_pricing: expect.objectContaining({ timezone: 'Asia/Shanghai' }),
    }))
    expect(getFinalPricing().time_pricing?.windows).toHaveLength(1)
  })

  it('auto-generates a distinct identifier for every window', async () => {
    const { root, getFinalPricing, getValidationError } = mountEditor({ tiers: BASE_TIERS } as TieredPricingConfig)

    button(root, 'time-pricing-enabled').click()
    await nextTick()
    button(root, 'time-pricing-add-window').click()
    await nextTick()
    // Adding a window reuses the default morning range, which legitimately overlaps window 1, so
    // move it to the afternoon peak before asserting the catalog is savable.
    typeInto(input(root, '[aria-label="第 2 个窗口的开始时间"]'), '14:00')
    typeInto(input(root, '[aria-label="第 2 个窗口的结束时间"]'), '18:00')
    await nextTick()

    expect(getValidationError()).toBeNull()
    const windows = getFinalPricing().time_pricing?.windows ?? []
    expect(windows).toHaveLength(2)
    expect(windows[0]?.id).toBeTruthy()
    expect(windows[1]?.id).toBeTruthy()
    expect(windows[0]?.id).not.toBe(windows[1]?.id)
  })

  it('builds a workday peak window from the editor and emits it to the form', async () => {
    const { root, onUpdate, getFinalPricing, getValidationError } = mountEditor({
      tiers: BASE_TIERS,
    } as TieredPricingConfig)
    expect(getValidationError()).toBeNull()

    button(root, 'time-pricing-enabled').click()
    await nextTick()
    expect(input(root, '[data-testid="time-pricing-timezone"]').value).toBe('Asia/Shanghai')
    expect(button(root, 'time-pricing-window-0-monday').getAttribute('aria-pressed')).toBe('true')
    expect(button(root, 'time-pricing-window-0-saturday').getAttribute('aria-pressed')).toBe('false')

    typeInto(input(root, '[data-testid="time-pricing-window-id"]'), 'workday-morning-peak')
    await nextTick()

    expect(getValidationError()).toBeNull()
    expect(getFinalPricing().time_pricing).toEqual(SHANGHAI_PEAK)
    expect(onUpdate).toHaveBeenLastCalledWith(expect.objectContaining({ time_pricing: SHANGHAI_PEAK }))
  })

  it('drops the key once the operator switches time pricing off', async () => {
    const { root, getFinalPricing } = mountEditor({
      tiers: BASE_TIERS,
      time_pricing: SHANGHAI_PEAK,
    } as TieredPricingConfig)

    button(root, 'time-pricing-enabled').click()
    await nextTick()

    expect('time_pricing' in getFinalPricing()).toBe(false)
  })

  it('rejects a window whose end is not after its start', async () => {
    const { root, getValidationError } = mountEditor({
      tiers: BASE_TIERS,
      time_pricing: SHANGHAI_PEAK,
    } as TieredPricingConfig)

    const endInput = input(root, '[aria-label="第 1 个窗口的结束时间"]')
    typeInto(endInput, '08:00')
    await nextTick()

    expect(getValidationError()).toBe('第 1 个时间窗口的结束时间必须晚于开始时间，且不支持跨午夜')
  })

  it('rejects overlapping windows that share a weekday', async () => {
    const { root, getValidationError } = mountEditor({
      tiers: BASE_TIERS,
      time_pricing: SHANGHAI_PEAK,
    } as TieredPricingConfig)

    button(root, 'time-pricing-add-window').click()
    await nextTick()
    typeInto(input(root, '[data-testid="time-pricing-window-id"]'), 'workday-morning-peak')
    await nextTick()
    const secondId = root.querySelectorAll('[data-testid="time-pricing-window-id"]')[1]
    if (!(secondId instanceof HTMLInputElement)) throw new Error('missing second window id')
    typeInto(secondId, 'workday-late-morning-peak')
    await nextTick()

    expect(getValidationError()).toBe('第 1 个时间窗口与第 2 个时间窗口存在重叠时段')
  })

  it('rejects duplicate window identifiers', async () => {
    const { root, getValidationError } = mountEditor({
      tiers: BASE_TIERS,
      time_pricing: {
        timezone: 'Asia/Shanghai',
        windows: [
          SHANGHAI_PEAK.windows[0],
          { ...SHANGHAI_PEAK.windows[0], start: '14:00', end: '18:00' },
        ],
      },
    } as TieredPricingConfig)

    typeInto(input(root, '[data-testid="time-pricing-window-id"]'), 'workday-morning-peak')
    await nextTick()

    expect(getValidationError()).toBe('时间窗口标识重复：workday-morning-peak')
  })

  it('rejects an empty timezone', async () => {
    const { root, getValidationError } = mountEditor({
      tiers: BASE_TIERS,
      time_pricing: SHANGHAI_PEAK,
    } as TieredPricingConfig)

    typeInto(input(root, '[data-testid="time-pricing-timezone"]'), '   ')
    await nextTick()

    expect(getValidationError()).toBe('分时定价必须填写时区，例如 Asia/Shanghai')
  })

  it('rejects an unknown IANA timezone and an excessive multiplier', async () => {
    const { root, getValidationError } = mountEditor({
      tiers: BASE_TIERS,
      time_pricing: SHANGHAI_PEAK,
    } as TieredPricingConfig)

    typeInto(input(root, '[data-testid="time-pricing-timezone"]'), 'Mars/Olympus')
    await nextTick()
    expect(getValidationError()).toBe('分时定价时区无效：Mars/Olympus')

    typeInto(input(root, '[data-testid="time-pricing-timezone"]'), 'Asia/Shanghai')
    typeInto(input(root, '[aria-label="第 1 个窗口的价格倍率"]'), '101')
    await nextTick()
    expect(getValidationError()).toBe('第 1 个时间窗口的倍率必须在 0 到 100 之间')
  })

  it('normalizes supported weekday abbreviations from existing catalogs', () => {
    const { getFinalPricing } = mountEditor({
      tiers: BASE_TIERS,
      time_pricing: {
        ...SHANGHAI_PEAK,
        windows: [{
          ...SHANGHAI_PEAK.windows[0],
          weekdays: ['mon', 'fri'],
        }],
      },
    } as TieredPricingConfig)

    expect(getFinalPricing().time_pricing?.windows[0]?.weekdays).toEqual(['monday', 'friday'])
  })
})
