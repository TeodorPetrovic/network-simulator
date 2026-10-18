DELETE FROM node_files
WHERE NOT EXISTS (
    SELECT 1
    FROM topology_objects
    WHERE topology_objects.project_id = node_files.project_id
      AND topology_objects.object_id = node_files.node_id
      AND topology_objects.kind = 'node'
);

CREATE TRIGGER node_files_cleanup_after_node_delete
AFTER DELETE ON topology_objects
FOR EACH ROW
WHEN OLD.kind = 'node'
BEGIN
    DELETE FROM node_files
    WHERE project_id = OLD.project_id AND node_id = OLD.object_id;
END;
