-- Add FK constraints linking all user_id columns to users(id).
-- Applied programmatically AFTER bootstrap_ownership() has rewritten 'default' rows.
-- NOT applied by the automatic refinery migration sweep.
ALTER TABLE conversations    ADD CONSTRAINT fk_conversations_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE memory_documents ADD CONSTRAINT fk_memory_documents_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE heartbeat_state  ADD CONSTRAINT fk_heartbeat_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE secrets          ADD CONSTRAINT fk_secrets_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE wasm_tools       ADD CONSTRAINT fk_wasm_tools_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE routines         ADD CONSTRAINT fk_routines_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE settings         ADD CONSTRAINT fk_settings_user
    FOREIGN KEY (user_id) REFERENCES users(id);
ALTER TABLE agent_jobs       ADD CONSTRAINT fk_agent_jobs_user
    FOREIGN KEY (user_id) REFERENCES users(id);
