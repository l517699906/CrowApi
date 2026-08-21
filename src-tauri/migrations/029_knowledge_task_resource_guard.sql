-- Prevent a late worker or retry from creating a task after its knowledge base
-- has been deleted. Existing task history remains available for auditing.
CREATE TRIGGER IF NOT EXISTS trg_background_tasks_knowledge_resource_insert
BEFORE INSERT ON background_tasks
WHEN NEW.domain = 'knowledge'
 AND NEW.resource_type = 'knowledge_base'
 AND NOT EXISTS (
     SELECT 1 FROM kb_knowledge_bases WHERE id = NEW.resource_id
 )
BEGIN
    SELECT RAISE(ABORT, 'knowledge base is not available');
END;

CREATE TRIGGER IF NOT EXISTS trg_background_tasks_knowledge_resource_update
BEFORE UPDATE OF domain, resource_type, resource_id ON background_tasks
WHEN NEW.domain = 'knowledge'
 AND NEW.resource_type = 'knowledge_base'
 AND NOT EXISTS (
     SELECT 1 FROM kb_knowledge_bases WHERE id = NEW.resource_id
 )
BEGIN
    SELECT RAISE(ABORT, 'knowledge base is not available');
END;
