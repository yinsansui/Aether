export type CsvValue = string | number | boolean | null | undefined

/** 所有字段统一加引号，并按 CSV 规则将内部双引号转义为两个双引号。 */
export function serializeCsv(rows: readonly (readonly CsvValue[])[]): string {
  return rows.map(row => row.map(value => {
    const text = value == null ? '' : String(value)
    return `"${text.replace(/"/g, '""')}"`
  }).join(',')).join('\r\n')
}
