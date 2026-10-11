/** 独立解析 CSV 字段，回归时验证实际列内容而非序列化字符串片段。 */
export function parseCsv(source: string): string[][] {
  const fieldPattern = /("(?:[^"]|"")*"|[^",\r\n]*)(,|\r\n|\n|$)/gy
  const rows: string[][] = []
  let row: string[] = []
  let offset = 0

  while (offset < source.length) {
    fieldPattern.lastIndex = offset
    const field = fieldPattern.exec(source)
    if (!field) throw new Error(`Invalid CSV field at ${offset}`)
    const value = field[1]!
    row.push(value.startsWith('"') ? value.slice(1, -1).replace(/""/g, '"') : value)
    offset = fieldPattern.lastIndex
    if (field[2] !== ',') {
      rows.push(row)
      row = []
    }
    if (field[2] === '') break
  }
  return rows
}
