// Matcher contract — frozen. Rule-based now, EmbeddingMatcher later.
export type TimeBucket = "S" | "M" | "L";
export type Energy = "LOW" | "MID" | "HIGH";

export interface PoolItem {
  id: string;
  skillNeeded: string[];
  timeBucket: TimeBucket;
  energy: Energy;
}

export interface QuestUser {
  id: string;
  canDo: string[];
  lookingFor: string[];
}

export interface RankedItem extends PoolItem {
  score: number;
  reason: string;
}

export interface Matcher {
  match(user: QuestUser, pool: PoolItem[]): RankedItem[];
}

function overlap(a: string[], b: string[]): number {
  if (a.length === 0 || b.length === 0) return 0;
  const setB = new Set(b.map((s) => s.toLowerCase()));
  const hits = a.filter((s) => setB.has(s.toLowerCase())).length;
  return hits / Math.max(a.length, b.length);
}

export class RuleBasedMatcher implements Matcher {
  match(user: QuestUser, pool: PoolItem[]): RankedItem[] {
    const others = pool.filter((p) => p.id !== user.id);
    const scored: RankedItem[] = others.map((p) => {
      const fit = (overlap(user.canDo, p.skillNeeded) + overlap(user.lookingFor, p.skillNeeded)) / 2;
      const random = Math.random();
      const score = Math.round((fit * 0.8 + random * 0.2) * 100) / 100;
      return { ...p, score, reason: `fit=${fit.toFixed(2)} rand=${random.toFixed(2)}` };
    });
    scored.sort((x, y) => y.score - x.score);
    const top3 = scored.slice(0, 3);
    const rest = scored.slice(3);
    const surprise = rest.length > 0 ? [rest[Math.floor(Math.random() * rest.length)]] : [];
    return [...top3, ...surprise];
  }
}
