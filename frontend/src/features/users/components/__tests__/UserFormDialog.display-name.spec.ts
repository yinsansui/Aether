import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, ref, type App } from 'vue'
import UserFormDialog, { type UserFormData } from '../UserFormDialog.vue'

vi.mock('@/api/admin', () => ({ adminApi: { getSystemConfig: vi.fn().mockResolvedValue({ value: 'weak' }) } }))
vi.mock('@/components/common', async () => {
  const { defineComponent, h } = await import('vue')
  return { MultiSelect: defineComponent({ setup: () => () => h('div') }) }
})
vi.mock('@/components/ui', async () => {
  const { defineComponent, h } = await import('vue')
  const container = defineComponent({ setup: (_, { slots }) => () => h('div', slots.default?.()) })
  const input = defineComponent({
    inheritAttrs: false,
    props: { modelValue: [String, Number] }, emits: ['update:modelValue'],
    setup: (props, { attrs, emit }) => () => h('input', {
      ...attrs, value: props.modelValue,
      onInput: (event: Event) => emit('update:modelValue', (event.target as HTMLInputElement).value),
    }),
  })
  return {
    Dialog: defineComponent({ setup: (_, { slots }) => () => h('div', [slots.header?.(), slots.default?.(), slots.footer?.()]) }),
    Button: defineComponent({ setup: (_, { slots }) => () => h('button', slots.default?.()) }),
    Input: input, Label: container, Switch: container, Select: container,
    SelectTrigger: container, SelectValue: container, SelectContent: container, SelectItem: container,
  }
})

const mounts: Array<{ app: App; root: HTMLElement }> = []
afterEach(() => mounts.splice(0).forEach(({ app, root }) => { app.unmount(); root.remove() }))

async function mountForm(user: UserFormData | null = null) {
  const open = ref(false)
  const submitted = vi.fn()
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp(defineComponent({ setup: () => () => h(UserFormDialog, { open: open.value, user, onSubmit: submitted }) }))
  app.mount(root)
  mounts.push({ app, root })
  open.value = true
  await nextTick()
  return { root, submitted }
}

async function input(root: HTMLElement, selector: string, value: string) {
  const element = root.querySelector<HTMLInputElement>(selector)!
  element.value = value
  element.dispatchEvent(new Event('input', { bubbles: true }))
  await nextTick()
}

const editingUser: UserFormData = { id: 'u1', username: 'zhangsan', display_name: '张三', email: '', role: 'user' }

describe('UserFormDialog display_name payload', () => {
  it('creates a trimmed Chinese name without changing the login account', async () => {
    const { root, submitted } = await mountForm()
    await input(root, '#form-username', 'zhangsan')
    await input(root, '#form-display-name', ' 张三 ')
    await input(root, 'input[id^="pwd-"]', 'password123')
    root.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    await nextTick()
    expect(submitted).toHaveBeenCalledWith(expect.objectContaining({ username: 'zhangsan', display_name: '张三' }))
  })

  it('sends null when an existing name is explicitly cleared', async () => {
    const { root, submitted } = await mountForm(editingUser)
    expect(root.querySelector<HTMLInputElement>('#form-display-name')!.value).toBe('张三')
    await input(root, '#form-display-name', '   ')
    root.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    expect(submitted).toHaveBeenCalledWith(expect.objectContaining({ id: 'u1', display_name: null }))
  })

  it('omits an unchanged name and blocks invalid names even on form submission', async () => {
    const { root, submitted } = await mountForm(editingUser)
    root.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    expect(submitted.mock.calls[0]![0]).not.toHaveProperty('display_name')
    submitted.mockClear()
    await input(root, '#form-display-name', '𠮷'.repeat(101))
    root.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
    expect(submitted).not.toHaveBeenCalled()
    expect(root.textContent).toContain('姓名不能超过100个字符')
  })
})
