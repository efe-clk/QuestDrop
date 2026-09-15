import { NextResponse } from "next/server";

// Placeholder: returns empty pool until DB is wired.
// Contract stays fixed so Bot (B) can reuse the same endpoint.
export async function GET() {
  return NextResponse.json({ pool: [] });
}
