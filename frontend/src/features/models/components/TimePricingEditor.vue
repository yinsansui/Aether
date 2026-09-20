<template>
  <div
    class="space-y-3 border-t border-border/60 pt-3"
    data-testid="time-pricing-editor"
  >
    <div class="flex items-start justify-between gap-3">
      <div class="min-w-0 space-y-1">
        <p class="text-sm font-medium text-foreground">
          分时定价
        </p>
        <p class="text-xs text-muted-foreground">
          按最终上游请求发出时刻判断所处时段，命中窗口的按倍率计价，其余时间按标准价。
        </p>
      </div>
      <Switch
        :model-value="enabled"
        data-testid="time-pricing-enabled"
        aria-label="启用分时定价"
        @update:model-value="setEnabled"
      />
    </div>

    <template v-if="enabled">
      <div class="space-y-1">
        <Label
          for="time-pricing-timezone"
          class="text-xs text-muted-foreground"
        >
          时区
        </Label>
        <Input
          id="time-pricing-timezone"
          v-model="timezone"
          list="time-pricing-timezone-options"
          class="h-8"
          placeholder="Asia/Shanghai"
          data-testid="time-pricing-timezone"
          @update:model-value="emitConfig"
        />
        <datalist id="time-pricing-timezone-options">
          <option
            v-for="zone in TIMEZONE_SUGGESTIONS"
            :key="zone"
            :value="zone"
          />
        </datalist>
      </div>

      <div class="space-y-2">
        <div
          v-for="(row, index) in rows"
          :key="row.key"
          class="space-y-2 rounded-md border border-border/60 p-2.5"
          :data-testid="`time-pricing-window-${index}`"
        >
          <div class="flex items-center gap-2">
            <Input
              v-model="row.id"
              class="h-8"
              placeholder="窗口标识，例如 workday-morning-peak"
              :aria-label="`第 ${index + 1} 个窗口的标识`"
              data-testid="time-pricing-window-id"
              @update:model-value="emitConfig"
            />
            <Button
              type="button"
              variant="ghost"
              size="icon"
              class="h-8 w-8 shrink-0"
              :aria-label="`删除第 ${index + 1} 个窗口`"
              @click="removeRow(index)"
            >
              <Trash2 class="h-4 w-4 text-muted-foreground hover:text-destructive" />
            </Button>
          </div>

          <div class="flex flex-wrap gap-1.5">
            <button
              v-for="weekday in WEEKDAYS"
              :key="weekday.value"
              type="button"
              class="rounded-md border px-2 py-1 text-xs transition-colors"
              :class="row.weekdays.includes(weekday.value)
                ? 'border-primary bg-primary/10 text-foreground'
                : 'border-border text-muted-foreground hover:text-foreground'"
              :aria-pressed="row.weekdays.includes(weekday.value)"
              :aria-label="`第 ${index + 1} 个窗口的${weekday.label}`"
              :data-testid="`time-pricing-window-${index}-${weekday.value}`"
              @click="toggleWeekday(row, weekday.value)"
            >
              {{ weekday.label }}
            </button>
          </div>

          <div class="flex flex-wrap items-end gap-2">
            <div class="space-y-1">
              <Label class="text-xs text-muted-foreground">开始</Label>
              <Input
                v-model="row.start"
                class="h-8 w-24"
                placeholder="09:00"
                :aria-label="`第 ${index + 1} 个窗口的开始时间`"
                @update:model-value="emitConfig"
              />
            </div>
            <div class="space-y-1">
              <Label class="text-xs text-muted-foreground">结束</Label>
              <Input
                v-model="row.end"
                class="h-8 w-24"
                placeholder="12:00"
                :aria-label="`第 ${index + 1} 个窗口的结束时间`"
                @update:model-value="emitConfig"
              />
            </div>
            <div class="space-y-1">
              <Label class="text-xs text-muted-foreground">倍率</Label>
              <Input
                v-model="row.multiplier"
                type="number"
                min="0"
                :max="MAX_TIME_PRICING_MULTIPLIER"
                step="0.1"
                class="h-8 w-24"
                :aria-label="`第 ${index + 1} 个窗口的价格倍率`"
                @update:model-value="emitConfig"
              />
            </div>
            <p class="pb-1.5 text-xs text-muted-foreground">
              结束时间不含在窗口内，例如 09:00-12:00 只把 12:00 之前算作高峰。
            </p>
          </div>
        </div>

        <Button
          type="button"
          variant="outline"
          size="sm"
          class="w-full"
          data-testid="time-pricing-add-window"
          @click="addRow"
        >
          <Plus class="w-4 h-4 mr-2" />
          添加时间窗口
        </Button>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import { Plus, Trash2 } from 'lucide-vue-next'
import { Button, Input, Label, Switch } from '@/components/ui'
import type { TimePricingConfig, TimePricingWindow } from '@/api/endpoints/types'

type WindowRow = {
  key: string
  id: string
  weekdays: string[]
  start: string
  end: string
  multiplier: string
}

const props = withDefaults(defineProps<{
  modelValue?: TimePricingConfig | null
}>(), {
  modelValue: null,
})
const emit = defineEmits<{
  'update:modelValue': [value: TimePricingConfig | null]
}>()

const WEEKDAYS = [
  { value: 'monday', label: '周一' },
  { value: 'tuesday', label: '周二' },
  { value: 'wednesday', label: '周三' },
  { value: 'thursday', label: '周四' },
  { value: 'friday', label: '周五' },
  { value: 'saturday', label: '周六' },
  { value: 'sunday', label: '周日' },
] as const
const WEEKDAY_VALUES: readonly string[] = WEEKDAYS.map(weekday => weekday.value)
const TIMEZONE_SUGGESTIONS = [
  'Asia/Shanghai',
  'Asia/Singapore',
  'Asia/Tokyo',
  'UTC',
  'America/New_York',
  'America/Los_Angeles',
  'Europe/London',
  'Europe/Berlin',
]
const DEFAULT_TIMEZONE = 'Asia/Shanghai'
const MAX_TIME_PRICING_MULTIPLIER = 100
const WEEKDAY_ALIASES: Record<string, string> = Object.fromEntries(
  WEEKDAYS.flatMap(weekday => [
    [weekday.value, weekday.value],
    [weekday.value.slice(0, 3), weekday.value],
  ]),
)

const enabled = ref(false)
const timezone = ref(DEFAULT_TIMEZONE)
const rows = ref<WindowRow[]>([])
const validationError = ref<string | null>(null)
let rowSequence = 0

watch(
  () => props.modelValue,
  (modelValue) => initializeFromModelValue(modelValue),
  { immediate: true },
)

function initializeFromModelValue(modelValue: TimePricingConfig | null | undefined) {
  const config = isRecord(modelValue) ? modelValue : null
  const windows = Array.isArray(config?.windows) ? config.windows : []
  if (!config || windows.length === 0) {
    enabled.value = false
    timezone.value = DEFAULT_TIMEZONE
    rows.value = []
    validationError.value = null
    return
  }
  enabled.value = true
  timezone.value = typeof config.timezone === 'string' && config.timezone.trim()
    ? config.timezone.trim()
    : DEFAULT_TIMEZONE
  rows.value = windows.map(window => rowFromWindow(window))
  validationError.value = null
}

function rowFromWindow(window: TimePricingWindow): WindowRow {
  return {
    key: nextRowKey(),
    id: typeof window?.id === 'string' ? window.id : '',
    weekdays: Array.isArray(window?.weekdays)
      ? [...new Set(window.weekdays
          .map((weekday) => {
            const normalized = String(weekday).trim().toLowerCase()
            return WEEKDAY_ALIASES[normalized] ?? normalized
          })
          .filter(Boolean))]
      : [],
    start: typeof window?.start === 'string' ? window.start : '',
    end: typeof window?.end === 'string' ? window.end : '',
    multiplier: formatMultiplier(window?.price_multiplier),
  }
}

function formatMultiplier(value: unknown): string {
  return typeof value === 'number' && Number.isFinite(value) ? String(value) : ''
}

function nextRowKey(): string {
  rowSequence += 1
  return `time-pricing-window-${rowSequence}`
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function setEnabled(value: boolean) {
  enabled.value = value
  if (value && rows.value.length === 0) {
    rows.value = [defaultRow()]
  }
  // Always emit: the parent form decides whether `time_pricing` is part of the saved catalog, so a
  // switch that never reaches it would be saved as still-off.
  emitConfig()
}

function defaultRow(): WindowRow {
  return {
    key: nextRowKey(),
    id: nextWindowId(),
    weekdays: WEEKDAY_VALUES.slice(0, 5),
    start: '09:00',
    end: '12:00',
    multiplier: '2',
  }
}

// The gateway rejects a window without an identifier, so a freshly added window has to carry one
// that is already usable instead of blocking the save until the operator names it.
function nextWindowId(): string {
  const used = new Set(rows.value.map(row => row.id.trim()).filter(Boolean))
  let index = 1
  while (used.has(`peak-${index}`)) {
    index += 1
  }
  return `peak-${index}`
}

function addRow() {
  rows.value = [...rows.value, defaultRow()]
  emitConfig()
}

function removeRow(index: number) {
  rows.value = rows.value.filter((_, rowIndex) => rowIndex !== index)
  emitConfig()
}

function toggleWeekday(row: WindowRow, weekday: string) {
  row.weekdays = row.weekdays.includes(weekday)
    ? row.weekdays.filter(value => value !== weekday)
    : WEEKDAY_VALUES.filter(value => row.weekdays.includes(value) || value === weekday)
  emitConfig()
}

function emitConfig() {
  validationError.value = validate()
  // Emit even while invalid so the parent keeps the user's edits; the parent's validation gate is
  // what stops an unevaluatable window from being saved.
  emit('update:modelValue', buildConfig())
}

function buildConfig(): TimePricingConfig | null {
  if (!enabled.value) return null
  return {
    timezone: timezone.value.trim(),
    windows: rows.value.map(row => ({
      id: row.id.trim(),
      weekdays: WEEKDAY_VALUES.filter(value => row.weekdays.includes(value)),
      start: row.start.trim(),
      end: row.end.trim(),
      price_multiplier: Number(row.multiplier),
    })),
  }
}

/**
 * Mirrors the gateway's `parse_time_pricing` so an unevaluatable window is caught while editing
 * instead of turning into a billing outage at settlement time.
 */
function validate(): string | null {
  if (!enabled.value) return null
  if (!timezone.value.trim()) return '分时定价必须填写时区，例如 Asia/Shanghai'
  try {
    new Intl.DateTimeFormat('en-US', { timeZone: timezone.value.trim() }).format()
  } catch {
    return `分时定价时区无效：${timezone.value.trim()}`
  }
  if (rows.value.length === 0) return '分时定价至少配置一个时间窗口'

  const seenIds = new Set<string>()
  for (const [index, row] of rows.value.entries()) {
    const position = `第 ${index + 1} 个时间窗口`
    const id = row.id.trim()
    if (!id) return `${position}的标识不能为空`
    if (seenIds.has(id)) return `时间窗口标识重复：${id}`
    seenIds.add(id)
    if (row.weekdays.length === 0) return `${position}至少选择一个星期`
    const unknownWeekday = row.weekdays.find(weekday => !WEEKDAY_VALUES.includes(weekday))
    if (unknownWeekday) return `${position}包含未知星期：${unknownWeekday}`
    const start = parseTimeOfDay(row.start)
    if (start === null) return `${position}的开始时间格式应为 HH:MM`
    const end = parseTimeOfDay(row.end)
    if (end === null) return `${position}的结束时间格式应为 HH:MM`
    if (end <= start) return `${position}的结束时间必须晚于开始时间，且不支持跨午夜`
    const multiplier = Number(row.multiplier)
    if (row.multiplier.trim() === '' || !Number.isFinite(multiplier)
      || multiplier < 0 || multiplier > MAX_TIME_PRICING_MULTIPLIER) {
      return `${position}的倍率必须在 0 到 ${MAX_TIME_PRICING_MULTIPLIER} 之间`
    }
  }

  return validateNoOverlap()
}

function validateNoOverlap(): string | null {
  for (let left = 0; left < rows.value.length; left += 1) {
    for (let right = left + 1; right < rows.value.length; right += 1) {
      const leftRow = rows.value[left]
      const rightRow = rows.value[right]
      if (!leftRow || !rightRow) continue
      const sharedWeekday = leftRow.weekdays.some(value => rightRow.weekdays.includes(value))
      if (!sharedWeekday) continue
      const leftStart = parseTimeOfDay(leftRow.start)
      const leftEnd = parseTimeOfDay(leftRow.end)
      const rightStart = parseTimeOfDay(rightRow.start)
      const rightEnd = parseTimeOfDay(rightRow.end)
      if (leftStart === null || leftEnd === null || rightStart === null || rightEnd === null) {
        continue
      }
      if (leftStart < rightEnd && rightStart < leftEnd) {
        return `第 ${left + 1} 个时间窗口与第 ${right + 1} 个时间窗口存在重叠时段`
      }
    }
  }
  return null
}

/** Returns the minute of day, accepting `24:00` as the end of the day like the gateway does. */
function parseTimeOfDay(value: string): number | null {
  const match = /^(\d{2}):(\d{2})$/.exec(value.trim())
  if (!match) return null
  const hour = Number(match[1])
  const minute = Number(match[2])
  if (hour > 24 || minute > 59) return null
  if (hour === 24 && minute !== 0) return null
  return hour * 60 + minute
}

defineExpose({
  getValidationError: () => validationError.value,
})
</script>
