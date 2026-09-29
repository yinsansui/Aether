<template>
  <Card class="overflow-hidden flex flex-col">
    <div class="px-3 py-2 border-b flex-shrink-0">
      <h3 class="text-sm font-medium">
        按模型分析
      </h3>
    </div>
    <div class="overflow-auto max-h-[320px]">
      <Table class="text-sm">
        <TableHeader>
          <TableRow>
            <TableHead class="h-8 px-2">
              模型
            </TableHead>
            <TableHead class="h-8 px-2 text-right">
              请求数
            </TableHead>
            <TableHead class="h-8 px-2 text-right">
              <div class="flex flex-col text-xs gap-0.5 whitespace-nowrap">
                <span>输入/输出</span>
                <span class="text-muted-foreground font-normal">缓存</span>
              </div>
            </TableHead>
            <TableHead class="h-8 px-2 text-right">
              费用
            </TableHead>
            <TableHead class="h-8 px-2 text-right">
              缓存命中率
            </TableHead>
            <TableHead class="h-8 px-2 text-right">
              效率
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow v-if="data.length === 0">
            <TableCell
              :colspan="6"
              class="text-center py-6 text-muted-foreground px-2"
            >
              暂无模型统计数据
            </TableCell>
          </TableRow>
          <TableRow
            v-for="model in data"
            :key="model.model"
          >
            <TableCell class="font-medium py-2 px-2">
              {{ model.model.replace('claude-', '') }}
            </TableCell>
            <TableCell class="text-right py-2 px-2">
              {{ model.request_count }}
            </TableCell>
            <TableCell class="text-right py-2 px-2">
              <div class="flex flex-col items-end text-xs gap-0.5 whitespace-nowrap">
                <span>{{ formatTokens(model.effective_input_tokens ?? model.total_input_context ?? 0) }} / {{ formatTokens(model.output_tokens || 0) }}</span>
                <span class="text-muted-foreground">{{ formatTokens(model.cache_read_tokens || 0) }}</span>
              </div>
            </TableCell>
            <TableCell class="text-right py-2 px-2">
              <div class="flex flex-col items-end text-xs gap-0.5">
                <span class="text-primary font-medium">{{ formatCurrency(primaryCost(model)) }}</span>
              </div>
            </TableCell>
            <TableCell class="text-right py-2 px-2">
              <span>{{ formatHitRate(model.cache_hit_rate) }}</span>
            </TableCell>
            <TableCell class="text-right text-muted-foreground py-2 px-2">
              {{ model.costPerToken }}
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </div>
  </Card>
</template>

<script setup lang="ts">
import Card from '@/components/ui/card.vue'
import Table from '@/components/ui/table.vue'
import TableHeader from '@/components/ui/table-header.vue'
import TableBody from '@/components/ui/table-body.vue'
import TableRow from '@/components/ui/table-row.vue'
import TableHead from '@/components/ui/table-head.vue'
import TableCell from '@/components/ui/table-cell.vue'
import { formatTokens, formatCurrency, formatHitRate } from '@/utils/format'
import { resolveCostDisplay } from '../utils/costDisplay'
import type { EnhancedModelStatsItem } from '../types'

defineProps<{
  data: EnhancedModelStatsItem[]
  isAdmin: boolean
}>()

// Charged amount is the only cost shown; it is what the wallet actually debits.
function primaryCost(item: { total_cost: number; actual_cost?: number }): number {
  return resolveCostDisplay(item.total_cost, item.actual_cost).primary
}

</script>
