// Which translation files a page needs; "common" (header, footer, shared parts) is always there.
const PAGES: Record<string, string[]> = {
  '': ['home', 'usecases', 'graph'],
  features: ['features'],
  'use-cases': ['usecases'],
  fred: ['fred', 'graph'],
  'how-it-works': ['how'],
  'why-bia': ['why'],
}

export function namespacesFor(path: string): string[] {
  const first = stripLocale(path).split(/[/?#]/)[1] || ''
  return ['common', ...(PAGES[first] ?? [first])]
}
