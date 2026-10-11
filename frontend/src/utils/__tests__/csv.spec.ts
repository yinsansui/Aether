import { describe, expect, it } from 'vitest'
import { serializeCsv } from '../csv'
import { parseCsv } from '@/tests/helpers/parseCsv'

describe('CSV serialization', () => {
  it('round-trips commas, double quotes and embedded line breaks through a CSV reader', () => {
    const rows = [
      ['姓名', '描述', '登录账号'],
      ['张"三,测试', '第一行\n第二行\r\n"备注"', 'zhangsan'],
      ['"首尾引号"', '', 'user,2'],
    ]
    expect(parseCsv(serializeCsv(rows))).toEqual(rows)
  })

  it('preserves zero and false while exporting absent values as empty cells', () => {
    expect(parseCsv(serializeCsv([[0, false, null, undefined, '']]))).toEqual([['0', 'false', '', '', '']])
    expect(serializeCsv([])).toBe('')
  })

  it('rejects the formerly emitted unescaped CSV rather than accepting shifted columns', () => {
    expect(() => parseCsv('"张"三,测试",zhangsan')).toThrow('Invalid CSV field')
  })
})
