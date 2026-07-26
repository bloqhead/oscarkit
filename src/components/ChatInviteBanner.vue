<script setup lang="ts">
import { useSession } from '../composables/useSession';

const { pendingInvites, acceptInvite, declineInvite } = useSession();
</script>

<template>
  <div v-if="pendingInvites.length" class="invite-stack">
    <div v-for="invite in pendingInvites" :key="`${invite.from}::${invite.room.room_cookie}`" class="invite-banner">
      <div class="invite-text">
        <strong>{{ invite.from }}</strong> invited you to <strong>{{ invite.room.room_name }}</strong>
        <div v-if="invite.invitation_text" class="invite-note">"{{ invite.invitation_text }}"</div>
      </div>
      <div class="invite-actions">
        <button class="btn-gold" type="button" @click="acceptInvite(invite)">Accept</button>
        <button class="btn-secondary" type="button" @click="declineInvite(invite)">Decline</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.invite-stack {
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.invite-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 12px;
  background: var(--away-banner-bg);
  border-bottom: 1px solid var(--away-banner-border);
  font-size: 12px;
}

.invite-text {
  min-width: 0;
  flex: 1;
}

.invite-note {
  color: #777;
  font-style: italic;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.invite-actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}
</style>
