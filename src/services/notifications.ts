export type NotificationItem = {
  id: number
  icon: 'mention' | 'security' | 'scheduled' | 'mail'
  title: string
  detail: string
  unread: boolean
}

const KEY = 'harbor-mail:notifications'

const seed: Omit<NotificationItem, 'unread'>[] = [
  {
    id: 1,
    icon: 'mention',
    title: 'Priya mentioned you',
    detail: 'Design review notes · 12 minutes ago',
  },
  {
    id: 2,
    icon: 'security',
    title: 'New sign-in detected',
    detail: 'Chrome on macOS · Today at 8:12 AM',
  },
  {
    id: 3,
    icon: 'scheduled',
    title: 'Scheduled message sent',
    detail: 'Q3 launch plan · Yesterday',
  },
]

export const notificationsApi = {
  load(): NotificationItem[] {
    try {
      const raw = localStorage.getItem(KEY)
      if (raw) return JSON.parse(raw) as NotificationItem[]
    } catch {
      /* ignore corrupt cache */
    }
    return seed.map((item) => ({ ...item, unread: true }))
  },
  save(items: NotificationItem[]) {
    localStorage.setItem(KEY, JSON.stringify(items))
  },
  add(item: Pick<NotificationItem, 'icon' | 'title' | 'detail'>) {
    this.save([{ ...item, id: Date.now(), unread: true }, ...this.load()])
  },
}
