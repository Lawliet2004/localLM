import type { VerticalProvider } from '../../types';
export interface CurrencyReport { baseCurrency: string; targetCurrency: string; amount: number; rate: number; convertedAmount: number; lastUpdated: string; sourceUrl: string }
export class CurrencyProvider implements VerticalProvider<string, CurrencyReport> {
  parseQuery(query: string): {amount: number; base: string; target: string} {
    const match = query.toUpperCase().match(/(?:(\d+(?:\.\d+)?)\s*)?\b([A-Z]{3})\s+(?:TO|IN)\s+([A-Z]{3})\b/);
    if (!match) throw new Error('Specify currencies, for example: 100 USD to EUR');
    return {amount: Number(match[1] || 1), base: match[2], target: match[3]};
  }
  async execute(query: string): Promise<CurrencyReport> {
    const {amount, base, target} = this.parseQuery(query);
    const sourceUrl = `https://open.er-api.com/v6/latest/${base}`;
    const res = await fetch(sourceUrl, {signal: AbortSignal.timeout(8000)});
    if (!res.ok) throw new Error(`Exchange rate unavailable (HTTP ${res.status})`);
    const data = await res.json();
    const rate = data.rates?.[target];
    if (!Number.isFinite(rate) || rate <= 0 || !data.time_last_update_utc) throw new Error('Exchange rate unavailable or invalid');
    return {baseCurrency:base,targetCurrency:target,amount,rate,convertedAmount:Number((amount*rate).toFixed(2)),lastUpdated:data.time_last_update_utc,sourceUrl};
  }
  formatReportAsEvidence(r: CurrencyReport): string {
    return `${r.amount} ${r.baseCurrency} = ${r.convertedAmount} ${r.targetCurrency} (1 ${r.baseCurrency} = ${r.rate} ${r.targetCurrency}, as of ${r.lastUpdated}; indicative rate from ExchangeRate-API).`;
  }
}
