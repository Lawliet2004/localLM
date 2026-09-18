/** Bounded global/per-domain semaphore. Wake all waiters to avoid cross-queue starvation. */
export class BoundedConcurrencyLimiter {
  private globalActive = 0;
  private domainActive = new Map<string, number>();
  private waiters: Array<() => void> = [];
  private maxGlobal: number;
  private maxPerDomain: number;
  constructor(maxGlobal = 8, maxPerDomain = 2) {
    if (!Number.isInteger(maxGlobal) || !Number.isInteger(maxPerDomain) || maxGlobal < 1 || maxPerDomain < 1) throw new Error('Concurrency must be positive integers');
    this.maxGlobal = maxGlobal; this.maxPerDomain = maxPerDomain;
  }
  async acquire(domain: string): Promise<() => void> {
    while (this.globalActive >= this.maxGlobal || (this.domainActive.get(domain) || 0) >= this.maxPerDomain) {
      await new Promise<void>(resolve => this.waiters.push(resolve));
    }
    this.globalActive++;
    this.domainActive.set(domain, (this.domainActive.get(domain) || 0) + 1);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.globalActive--;
      const active = (this.domainActive.get(domain) || 1) - 1;
      if (active) this.domainActive.set(domain, active); else this.domainActive.delete(domain);
      const waiters = this.waiters.splice(0);
      waiters.forEach(resolve => resolve());
    };
  }
}
