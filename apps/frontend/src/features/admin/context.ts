import { computed, inject, type ComputedRef, type InjectionKey, type Ref } from 'vue'
import type { SessionUser } from '@/api/contracts'

/**
 * Signed-in administrator identity shared by every admin page.
 */
export interface AdminContext {
  user: Ref<SessionUser | null>

  /**
   * Owners and admins manage users, scopes, and SMTP access.
   */
  isGlobalAdmin: ComputedRef<boolean>
  isOwner: ComputedRef<boolean>
}

export const adminContextKey: InjectionKey<AdminContext> = Symbol('AdminContext')

/**
 * Build the context exposed by the admin layout.
 */
export function createAdminContext(user: Ref<SessionUser | null>): AdminContext {
  return {
    user,
    isGlobalAdmin: computed(() => user.value?.role === 'owner' || user.value?.role === 'admin'),
    isOwner: computed(() => user.value?.role === 'owner'),
  }
}

/**
 * Return the admin layout's context.
 */
export function useAdminContext(): AdminContext {
  const context = inject(adminContextKey)
  if (!context) throw new Error('Admin pages must render inside the admin layout')
  return context
}
